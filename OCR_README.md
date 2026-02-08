# Universal OCR Tool

Modular OCR interface supporting 7 providers, including specialized solutions for mathematical formulas.

## Supported Providers

### General OCR
| Provider | Description | Best For |
|-----------|-------------|----------|
| `mistral` | Mistral OCR API | Documents, complex layouts |
| `kimi` | Moonshot AI Vision | Chinese text, documents |

### Math OCR (STEM)
| Provider | Description | Features |
|-----------|-------------|----------|
| `mathpix` | **Best for math** | LaTeX output, chemical formulas, SMILES |
| `azure` | Azure Document Intelligence | LaTeX formulas, tables, handwriting |
| `google` | Google Document AI | Math OCR addon, font detection |
| `openai` | GPT-4o Vision | Universal, LaTeX formulas |
| `claude` | Anthropic Claude Vision | Structured output, tables |

## Installation

```bash
# Activate virtual environment
source .venv/bin/activate

# Install dependencies (already installed)
pip install mistralai openai anthropic azure-ai-documentintelligence google-cloud-documentai
```

## API Key Configuration

Copy `.env.example` to `.env` and fill in your keys:

```bash
cp .env.example .env
```

### Environment Variables

```env
# General OCR
MISTRAL_API_KEY=your_key
KIMI_API_KEY=your_key

# For Math
MATHPIX_API_KEY=your_key
MATHPIX_APP_ID=your_app_id

AZURE_API_KEY=your_key
AZURE_ENDPOINT=https://your-resource.cognitiveservices.azure.com

GOOGLE_PROJECT_ID=your_project_id
GOOGLE_PROCESSOR_ID=your_processor_id

OPENAI_API_KEY=your_key
ANTHROPIC_API_KEY=your_key
```

## Usage

### Basic Examples

```bash
# Mistral OCR (default)
python ocr.py document.pdf

# Mathpix - best for mathematical formulas
python ocr.py math_equation.png -p mathpix

# Azure Document Intelligence with formulas
python ocr.py document.pdf -p azure --extract-formulas

# GPT-4o Vision
python ocr.py document.pdf -p openai

# Claude Vision
python ocr.py document.png -p claude

# Save result to file
python ocr.py doc.pdf -p mathpix -o result.txt
```

### Advanced Examples

```bash
# Mathpix with additional formats
python ocr.py formula.png -p mathpix \
  --mathpix-app-id your_app_id \
  --prompt "Extract with LaTeX and AsciiMath"

# Azure with custom endpoint
python ocr.py document.pdf -p azure \
  --azure-endpoint https://your-resource.cognitiveservices.azure.com \
  --extract-formulas

# OpenAI with custom prompt
python ocr.py math.png -p openai \
  --openai-model gpt-4o \
  --prompt "Extract all text and convert math to LaTeX"

# Claude with custom prompt
python ocr.py table.png -p claude \
  --claude-model claude-3-5-sonnet-20241022 \
  --prompt "Extract table as markdown"
```

## Python API

```python
from ocr import OCRFactory

# Create provider
provider = OCRFactory.create("mathpix")

# Process document
result = provider.process("document.png")
print(result)

# With custom settings
provider = OCRFactory.create(
    "azure",
    api_key="your_key",
    endpoint="https://your-resource.cognitiveservices.azure.com"
)
result = provider.process("math.pdf", extract_formulas=True)
```

## Math OCR Provider Comparison

### Mathpix
- ✅ Best accuracy for math
- ✅ Supports LaTeX, AsciiMath, MathML
- ✅ Chemical diagrams (SMILES)
- ✅ Handwritten formulas
- ❌ Paid (per request)

### Azure Document Intelligence
- ✅ Native LaTeX formula support
- ✅ Excellent tables
- ✅ Microsoft ecosystem integration
- ✅ Enterprise privacy
- ❌ Requires Azure subscription

### Google Document AI
- ✅ Math OCR addon
- ✅ Font style detection
- ✅ Image quality scores
- ✅ 200+ languages
- ❌ Complex setup (processor ID)

### GPT-4o / Claude
- ✅ Universal
- ✅ Good context understanding
- ✅ No special setup required
- ❌ Slower than specialized OCR
- ❌ May hallucinate

## Use Cases

| Task | Recommended Provider |
|------|---------------------|
| Mathematical papers | **Mathpix** |
| Scientific publications with formulas | **Mathpix** or **Azure** |
| Tables and forms | **Azure** or **Claude** |
| Handwritten notes | **Mathpix** or **Google** |
| Multilingual documents | **Google** or **Mistral** |
| Quick prototype | **OpenAI** or **Claude** |
| Enterprise/confidentiality | **Azure** or **Google** |

## Recommendations

1. **Math only** → Mathpix
2. **Math + tables + enterprise** → Azure
3. **Budget all-in-one solution** → GPT-4o
4. **Best structured output** → Claude
5. **GCP integration** → Google Document AI
