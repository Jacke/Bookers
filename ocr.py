#!/usr/bin/env python3
"""
Universal OCR interface supporting multiple providers including specialized math OCR.

Supported providers:
- Mistral OCR
- Kimi (Moonshot AI) Vision
- Mathpix (best for math/formulas)
- Azure Document Intelligence (with formula extraction)
- Google Document AI (with math OCR)
- OpenAI GPT-4o Vision
- Anthropic Claude Vision
"""

import os
import sys
import argparse
import base64
import json
import time
from abc import ABC, abstractmethod
from typing import Optional, Union, Dict, Any, List
from pathlib import Path


class OCRProvider(ABC):
    """Base class for OCR providers."""
    
    def __init__(self, api_key: str):
        self.api_key = api_key
    
    @abstractmethod
    def process(self, document: Union[str, Path], **kwargs) -> str:
        """
        Process document and return OCR text.
        
        Args:
            document: URL string or Path to local file
            **kwargs: Additional provider-specific options
            
        Returns:
            Extracted text from document
        """
        pass
    
    @staticmethod
    def encode_file_to_base64(file_path: Path) -> str:
        """Encode local file to base64 string."""
        with open(file_path, "rb") as f:
            return base64.b64encode(f.read()).decode("utf-8")


class MistralOCRProvider(OCRProvider):
    """Mistral OCR API provider."""
    
    def __init__(self, api_key: str):
        super().__init__(api_key)
        try:
            from mistralai import Mistral
        except ImportError:
            raise ImportError("mistralai package required. Install: pip install mistralai")
        self.client = Mistral(api_key=api_key)
    
    def process(self, document: Union[str, Path], include_images: bool = True, **kwargs) -> str:
        """Process document using Mistral OCR."""
        from mistralai import DocumentURLChunk
        
        # Determine if document is URL or local file
        if isinstance(document, Path) or (
            isinstance(document, str) and not document.startswith(("http://", "https://"))
        ):
            # Local file - upload first
            file_path = Path(document)
            if not file_path.exists():
                raise FileNotFoundError(f"File not found: {file_path}")
            
            uploaded_file = self.client.files.upload(
                file={
                    "file_name": file_path.name,
                    "content": file_path.read_bytes(),
                },
                purpose="ocr"
            )
            
            signed_url = self.client.files.get_signed_url(file_id=uploaded_file.id, expiry=1)
            document_url = signed_url.url
        else:
            # Remote URL
            document_url = str(document)
        
        # Process OCR
        ocr_response = self.client.ocr.process(
            model="mistral-ocr-latest",
            document={
                "type": "document_url",
                "document_url": document_url,
            },
            include_image_base64=include_images
        )
        
        # Extract text from all pages
        texts = []
        for page in ocr_response.pages:
            texts.append(page.markdown)
        
        return "\n\n".join(texts)


class KimiOCRProvider(OCRProvider):
    """Kimi (Moonshot AI) Vision OCR provider."""
    
    API_BASE = "https://api.moonshot.cn/v1"
    
    def __init__(self, api_key: str):
        super().__init__(api_key)
        try:
            from openai import OpenAI
        except ImportError:
            raise ImportError("openai package required. Install: pip install openai")
        
        self.client = OpenAI(
            api_key=api_key,
            base_url=self.API_BASE
        )
    
    def process(
        self, 
        document: Union[str, Path], 
        model: str = "kimi-k2",
        prompt: Optional[str] = None,
        **kwargs
    ) -> str:
        """Process document using Kimi Vision API."""
        
        if prompt is None:
            prompt = "Extract all text content from this document. Preserve the structure and formatting as much as possible."
        
        # Determine if document is URL or local file
        doc_url = None
        local_path = None
        
        if isinstance(document, Path):
            local_path = document
        elif isinstance(document, str):
            if document.startswith(("http://", "https://")):
                doc_url = document
            else:
                local_path = Path(document)
        
        # Prepare content
        if local_path:
            if not local_path.exists():
                raise FileNotFoundError(f"File not found: {local_path}")
            
            # For local files, we need to use base64
            file_ext = local_path.suffix.lower()
            mime_types = {
                ".pdf": "application/pdf",
                ".png": "image/png",
                ".jpg": "image/jpeg",
                ".jpeg": "image/jpeg",
                ".gif": "image/gif",
                ".webp": "image/webp",
            }
            mime_type = mime_types.get(file_ext, "application/octet-stream")
            
            base64_content = self.encode_file_to_base64(local_path)
            image_url = f"data:{mime_type};base64,{base64_content}"
        else:
            # Remote URL
            image_url = doc_url
        
        # Make API call
        messages = [
            {
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": image_url}},
                    {"type": "text", "text": prompt}
                ]
            }
        ]
        
        response = self.client.chat.completions.create(
            model=model,
            messages=messages,
            temperature=0.1,
        )
        
        return response.choices[0].message.content


class MathpixOCRProvider(OCRProvider):
    """
    Mathpix OCR provider - best for math, science, and STEM documents.
    Returns LaTeX for formulas, supports handwritten and printed math.
    """
    
    API_BASE = "https://api.mathpix.com/v3"
    
    def __init__(self, api_key: str, app_id: Optional[str] = None):
        """
        Initialize Mathpix OCR.
        
        Args:
            api_key: Mathpix API key
            app_id: Mathpix App ID (if None, uses MATHPIX_APP_ID env var)
        """
        self.api_key = api_key
        self.app_id = app_id or os.environ.get("MATHPIX_APP_ID")
        if not self.app_id:
            raise ValueError("Mathpix requires app_id. Set MATHPIX_APP_ID env var or pass app_id parameter.")
    
    def process(
        self, 
        document: Union[str, Path],
        formats: Optional[List[str]] = None,
        include_latex: bool = True,
        include_asciimath: bool = False,
        include_mathml: bool = False,
        **kwargs
    ) -> str:
        """
        Process document using Mathpix OCR.
        
        Args:
            document: Path to image/PDF or URL
            formats: Output formats (text, latex_styled, data, html)
            include_latex: Include LaTeX in data output
            include_asciimath: Include AsciiMath in data output
            include_mathml: Include MathML in data output
        """
        import requests
        
        if formats is None:
            formats = ["text", "data"]
        
        # Build request
        data_options = {}
        if include_latex:
            data_options["include_latex"] = True
        if include_asciimath:
            data_options["include_asciimath"] = True
        if include_mathml:
            data_options["include_mathml"] = True
        
        headers = {
            "app_id": self.app_id,
            "app_key": self.api_key,
            "Content-type": "application/json"
        }
        
        # Determine if document is URL or local file
        is_url = isinstance(document, str) and document.startswith(("http://", "https://"))
        
        if is_url:
            # Process URL
            payload = {
                "src": document,
                "formats": formats,
            }
            if data_options:
                payload["data_options"] = data_options
            
            response = requests.post(
                f"{self.API_BASE}/text",
                json=payload,
                headers=headers
            )
        else:
            # Process local file
            file_path = Path(document)
            if not file_path.exists():
                raise FileNotFoundError(f"File not found: {file_path}")
            
            options_json = json.dumps({"formats": formats})
            if data_options:
                options_json = json.dumps({"formats": formats, "data_options": data_options})
            
            with open(file_path, "rb") as f:
                response = requests.post(
                    f"{self.API_BASE}/text",
                    files={"file": f},
                    data={"options_json": options_json},
                    headers=headers
                )
        
        response.raise_for_status()
        result = response.json()
        
        # Return text (includes LaTeX in markdown format)
        return result.get("text", "")


class AzureDocumentIntelligenceProvider(OCRProvider):
    """
    Azure Document Intelligence OCR provider.
    Supports formula extraction with LaTeX output.
    """
    
    def __init__(self, api_key: str, endpoint: Optional[str] = None):
        """
        Initialize Azure Document Intelligence.
        
        Args:
            api_key: Azure API key
            endpoint: Azure endpoint URL (if None, uses AZURE_ENDPOINT env var)
        """
        super().__init__(api_key)
        self.endpoint = endpoint or os.environ.get("AZURE_ENDPOINT")
        if not self.endpoint:
            raise ValueError("Azure requires endpoint. Set AZURE_ENDPOINT env var or pass endpoint parameter.")
        
        try:
            from azure.ai.documentintelligence import DocumentIntelligenceClient
            from azure.core.credentials import AzureKeyCredential
        except ImportError:
            raise ImportError("azure-ai-documentintelligence package required. Install: pip install azure-ai-documentintelligence")
        
        self.client = DocumentIntelligenceClient(
            endpoint=self.endpoint,
            credential=AzureKeyCredential(api_key)
        )
    
    def process(
        self, 
        document: Union[str, Path],
        extract_formulas: bool = True,
        extract_tables: bool = True,
        **kwargs
    ) -> str:
        """
        Process document using Azure Document Intelligence.
        
        Args:
            document: Path to file or URL
            extract_formulas: Enable formula extraction (LaTeX)
            extract_tables: Enable table extraction
        """
        from azure.ai.documentintelligence.models import AnalyzeResult
        
        # Build features list
        features = []
        if extract_formulas:
            features.append("formulas")
        if extract_tables:
            features.append("keyValuePairs")
        
        # Determine if URL or local file
        is_url = isinstance(document, str) and document.startswith(("http://", "https://"))
        
        if is_url:
            # Process URL
            poller = self.client.begin_analyze_document(
                "prebuilt-layout",
                {"urlSource": document},
                features=features if features else None
            )
        else:
            # Process local file
            file_path = Path(document)
            if not file_path.exists():
                raise FileNotFoundError(f"File not found: {file_path}")
            
            with open(file_path, "rb") as f:
                poller = self.client.begin_analyze_document(
                    "prebuilt-layout",
                    f,
                    features=features if features else None
                )
        
        result = poller.result()
        
        # Extract text
        texts = []
        
        if result.pages:
            for page in result.pages:
                if page.lines:
                    for line in page.lines:
                        texts.append(line.content)
        
        # Extract formulas if available
        if extract_formulas and hasattr(result, 'formulas') and result.formulas:
            texts.append("\n--- Formulas (LaTeX) ---")
            for formula in result.formulas:
                texts.append(f"[{formula.kind}]: {formula.value}")
        
        return "\n".join(texts)


class GoogleDocumentAIOCRProvider(OCRProvider):
    """
    Google Document AI OCR provider.
    Supports math formula extraction with LaTeX output.
    """
    
    def __init__(self, api_key: Optional[str] = None, project_id: Optional[str] = None, location: str = "us"):
        """
        Initialize Google Document AI.
        
        Args:
            api_key: Google API key (uses GOOGLE_APPLICATION_CREDENTIALS if None)
            project_id: GCP Project ID
            location: Processor location (us or eu)
        """
        # Google uses service account credentials, not API key
        self.project_id = project_id or os.environ.get("GOOGLE_PROJECT_ID")
        self.location = location
        
        try:
            from google.cloud import documentai
            from google.api_core.client_options import ClientOptions
        except ImportError:
            raise ImportError("google-cloud-documentai package required. Install: pip install google-cloud-documentai")
        
        opts = ClientOptions(api_endpoint=f"{location}-documentai.googleapis.com")
        self.client = documentai.DocumentProcessorServiceClient(client_options=opts)
    
    def process(
        self, 
        document: Union[str, Path],
        processor_id: Optional[str] = None,
        enable_math_ocr: bool = True,
        **kwargs
    ) -> str:
        """
        Process document using Google Document AI.
        
        Args:
            document: Path to file
            processor_id: Document AI processor ID (uses GOOGLE_PROCESSOR_ID if None)
            enable_math_ocr: Enable math formula extraction
        """
        from google.cloud import documentai
        
        processor_id = processor_id or os.environ.get("GOOGLE_PROCESSOR_ID")
        if not processor_id:
            raise ValueError("Google Document AI requires processor_id. Set GOOGLE_PROCESSOR_ID env var or pass processor_id parameter.")
        
        if not self.project_id:
            raise ValueError("Google Document AI requires project_id. Set GOOGLE_PROJECT_ID env var.")
        
        # Resource name
        name = self.client.processor_path(self.project_id, self.location, processor_id)
        
        # Read document
        file_path = Path(document)
        if not file_path.exists():
            raise FileNotFoundError(f"File not found: {file_path}")
        
        with open(file_path, "rb") as f:
            image_content = f.read()
        
        # Mime type
        mime_types = {
            ".pdf": "application/pdf",
            ".png": "image/png",
            ".jpg": "image/jpeg",
            ".jpeg": "image/jpeg",
            ".tiff": "image/tiff",
            ".gif": "image/gif",
        }
        mime_type = mime_types.get(file_path.suffix.lower(), "application/pdf")
        
        raw_document = documentai.RawDocument(content=image_content, mime_type=mime_type)
        
        # Configure options
        process_options = None
        if enable_math_ocr:
            process_options = documentai.ProcessOptions(
                ocr_config=documentai.OcrConfig(
                    premium_features=documentai.OcrConfig.PremiumFeatures(
                        enable_math_ocr=True
                    )
                )
            )
        
        request = documentai.ProcessRequest(
            name=name,
            raw_document=raw_document,
            process_options=process_options
        )
        
        result = self.client.process_document(request=request)
        
        # Extract text
        document = result.document
        texts = [document.text]
        
        # Extract math formulas if available
        if enable_math_ocr and document.pages:
            texts.append("\n--- Math Formulas ---")
            for page in document.pages:
                if hasattr(page, 'visual_elements'):
                    for element in page.visual_elements:
                        if element.type == "math_formula":
                            # Get text anchor content
                            if element.layout and element.layout.text_anchor:
                                text_segments = element.layout.text_anchor.text_segments
                                formula_text = ""
                                for segment in text_segments:
                                    start = int(segment.start_index) if segment.start_index else 0
                                    end = int(segment.end_index) if segment.end_index else 0
                                    formula_text += document.text[start:end]
                                texts.append(f"[Formula]: {formula_text}")
        
        return "\n".join(texts)


class OpenAIVisionOCRProvider(OCRProvider):
    """
    OpenAI GPT-4o Vision OCR provider.
    Excellent for math formulas and complex layouts.
    """
    
    def __init__(self, api_key: str):
        super().__init__(api_key)
        try:
            from openai import OpenAI
        except ImportError:
            raise ImportError("openai package required. Install: pip install openai")
        
        self.client = OpenAI(api_key=api_key)
    
    def process(
        self, 
        document: Union[str, Path],
        model: str = "gpt-4o",
        prompt: Optional[str] = None,
        **kwargs
    ) -> str:
        """
        Process document using OpenAI GPT-4o Vision.
        
        Args:
            document: Path to file or URL
            model: Model name (gpt-4o, gpt-4o-mini, gpt-4-turbo)
            prompt: Custom prompt for extraction
        """
        if prompt is None:
            prompt = (
                "Extract all text from this document. "
                "For mathematical formulas, use LaTeX format inside $$ delimiters. "
                "Preserve the document structure and formatting."
            )
        
        # Determine if document is URL or local file
        is_url = isinstance(document, str) and document.startswith(("http://", "https://"))
        
        if is_url:
            image_url = document
        else:
            file_path = Path(document)
            if not file_path.exists():
                raise FileNotFoundError(f"File not found: {file_path}")
            
            base64_content = self.encode_file_to_base64(file_path)
            
            # Determine mime type
            mime_types = {
                ".png": "image/png",
                ".jpg": "image/jpeg",
                ".jpeg": "image/jpeg",
                ".gif": "image/gif",
                ".webp": "image/webp",
            }
            mime_type = mime_types.get(file_path.suffix.lower(), "image/png")
            image_url = f"data:{mime_type};base64,{base64_content}"
        
        messages = [
            {
                "role": "user",
                "content": [
                    {
                        "type": "image_url",
                        "image_url": {"url": image_url}
                    },
                    {
                        "type": "text",
                        "text": prompt
                    }
                ]
            }
        ]
        
        response = self.client.chat.completions.create(
            model=model,
            messages=messages,
            temperature=0.1,
            max_tokens=4096
        )
        
        return response.choices[0].message.content


class ClaudeVisionOCRProvider(OCRProvider):
    """
    Anthropic Claude Vision OCR provider.
    Excellent for structured documents and math formulas.
    """
    
    def __init__(self, api_key: str):
        super().__init__(api_key)
        try:
            import anthropic
        except ImportError:
            raise ImportError("anthropic package required. Install: pip install anthropic")
        
        self.client = anthropic.Anthropic(api_key=api_key)
    
    def process(
        self, 
        document: Union[str, Path],
        model: str = "claude-3-5-sonnet-20241022",
        prompt: Optional[str] = None,
        **kwargs
    ) -> str:
        """
        Process document using Anthropic Claude Vision.
        
        Args:
            document: Path to file or URL
            model: Model name (claude-3-5-sonnet, claude-3-opus, etc.)
            prompt: Custom prompt for extraction
        """
        if prompt is None:
            prompt = (
                "Extract all text from this image. "
                "Convert mathematical formulas to LaTeX format. "
                "Preserve document structure including tables and formatting. "
                "Return only the extracted content without any additional commentary."
            )
        
        # Determine if document is URL or local file
        is_url = isinstance(document, str) and document.startswith(("http://", "https://"))
        
        if is_url:
            # Download image from URL
            import requests
            response = requests.get(document)
            image_data = base64.b64encode(response.content).decode("utf-8")
            
            # Determine mime type from content
            content_type = response.headers.get('content-type', 'image/jpeg')
            media_type = content_type if 'image/' in content_type else 'image/jpeg'
        else:
            file_path = Path(document)
            if not file_path.exists():
                raise FileNotFoundError(f"File not found: {file_path}")
            
            image_data = self.encode_file_to_base64(file_path)
            
            # Determine mime type
            mime_types = {
                ".png": "image/png",
                ".jpg": "image/jpeg",
                ".jpeg": "image/jpeg",
                ".gif": "image/gif",
                ".webp": "image/webp",
            }
            media_type = mime_types.get(file_path.suffix.lower(), "image/jpeg")
        
        message = self.client.messages.create(
            model=model,
            max_tokens=4096,
            messages=[
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": media_type,
                                "data": image_data
                            }
                        },
                        {
                            "type": "text",
                            "text": prompt
                        }
                    ]
                }
            ]
        )
        
        return message.content[0].text


class OCRFactory:
    """Factory for creating OCR providers."""
    
    _providers = {
        "mistral": MistralOCRProvider,
        "kimi": KimiOCRProvider,
        "mathpix": MathpixOCRProvider,
        "azure": AzureDocumentIntelligenceProvider,
        "google": GoogleDocumentAIOCRProvider,
        "openai": OpenAIVisionOCRProvider,
        "claude": ClaudeVisionOCRProvider,
    }
    
    @classmethod
    def create(cls, provider_name: str, api_key: Optional[str] = None, **kwargs) -> OCRProvider:
        """
        Create OCR provider instance.
        
        Args:
            provider_name: Name of the provider
            api_key: API key (if None, will try to get from environment)
            **kwargs: Additional provider-specific arguments
            
        Returns:
            OCRProvider instance
        """
        provider_name = provider_name.lower()
        
        if provider_name not in cls._providers:
            raise ValueError(
                f"Unknown provider: {provider_name}. "
                f"Available: {', '.join(cls._providers.keys())}"
            )
        
        # Get API key from environment if not provided
        if api_key is None:
            env_vars = {
                "mistral": "MISTRAL_API_KEY",
                "kimi": "KIMI_API_KEY",
                "mathpix": "MATHPIX_API_KEY",
                "azure": "AZURE_API_KEY",
                "google": "GOOGLE_APPLICATION_CREDENTIALS",  # Special case
                "openai": "OPENAI_API_KEY",
                "claude": "ANTHROPIC_API_KEY",
            }
            env_var = env_vars[provider_name]
            api_key = os.environ.get(env_var)
            
            if not api_key and provider_name != "google":
                raise ValueError(
                    f"API key not provided and {env_var} "
                    f"environment variable not set."
                )
        
        provider_class = cls._providers[provider_name]
        
        # Special handling for providers that need extra params
        if provider_name == "mathpix":
            return provider_class(api_key=api_key, app_id=kwargs.get("app_id"))
        elif provider_name == "azure":
            return provider_class(api_key=api_key, endpoint=kwargs.get("endpoint"))
        elif provider_name == "google":
            return provider_class(
                project_id=kwargs.get("project_id"),
                location=kwargs.get("location", "us")
            )
        
        return provider_class(api_key)
    
    @classmethod
    def list_providers(cls) -> List[str]:
        """Return list of available provider names."""
        return list(cls._providers.keys())
    
    @classmethod
    def register(cls, name: str, provider_class: type[OCRProvider]):
        """Register a new provider."""
        cls._providers[name.lower()] = provider_class


def main():
    parser = argparse.ArgumentParser(
        description="Universal OCR tool supporting multiple providers."
    )
    parser.add_argument(
        "document",
        help="Path to local file or URL to the document"
    )
    parser.add_argument(
        "-p", "--provider",
        choices=OCRFactory.list_providers(),
        default="mistral",
        help="OCR provider to use (default: mistral)"
    )
    parser.add_argument(
        "-k", "--api-key",
        help="API key (if not set, will use environment variable)"
    )
    parser.add_argument(
        "-o", "--output",
        help="Output file path (default: print to stdout)"
    )
    
    # Provider-specific options
    parser.add_argument(
        "--kimi-model",
        default="kimi-k2",
        help="Kimi model to use (default: kimi-k2)"
    )
    parser.add_argument(
        "--openai-model",
        default="gpt-4o",
        help="OpenAI model to use (default: gpt-4o)"
    )
    parser.add_argument(
        "--claude-model",
        default="claude-3-5-sonnet-20241022",
        help="Claude model to use (default: claude-3-5-sonnet-20241022)"
    )
    parser.add_argument(
        "--mathpix-app-id",
        help="Mathpix App ID (required for Mathpix)"
    )
    parser.add_argument(
        "--azure-endpoint",
        help="Azure Document Intelligence endpoint URL"
    )
    parser.add_argument(
        "--prompt",
        help="Custom prompt for vision-based OCR"
    )
    parser.add_argument(
        "--extract-formulas", 
        action="store_true",
        default=True,
        help="Extract mathematical formulas (for Azure/Google)"
    )
    
    args = parser.parse_args()
    
    try:
        # Create provider with extra kwargs
        kwargs = {}
        if args.provider == "kimi":
            kwargs["model"] = args.kimi_model
        elif args.provider == "openai":
            kwargs["model"] = args.openai_model
        elif args.provider == "claude":
            kwargs["model"] = args.claude_model
        elif args.provider == "mathpix":
            if args.mathpix_app_id:
                kwargs["app_id"] = args.mathpix_app_id
        elif args.provider == "azure":
            if args.azure_endpoint:
                kwargs["endpoint"] = args.azure_endpoint
            kwargs["extract_formulas"] = args.extract_formulas
        elif args.provider == "google":
            kwargs["enable_math_ocr"] = args.extract_formulas
        
        if args.prompt:
            kwargs["prompt"] = args.prompt
        
        provider = OCRFactory.create(args.provider, args.api_key, **kwargs)
        
        # Process document
        result = provider.process(args.document, **kwargs)
        
        # Output result
        if args.output:
            Path(args.output).write_text(result, encoding="utf-8")
            print(f"OCR result saved to: {args.output}")
        else:
            print(result)
            
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
