import os
import argparse
from mistralai import Mistral

def main():
    # Setup CLI argument parsing
    parser = argparse.ArgumentParser(description="Run OCR on a remote PDF using Mistral.")
    parser.add_argument("url", help="Public URL to the PDF file")
    args = parser.parse_args()

    # Load API key from environment
    api_key = os.environ.get("MISTRAL_API_KEY")
    if not api_key:
        raise ValueError("MISTRAL_API_KEY environment variable not set.")

    # Initialize Mistral client
    client = Mistral(api_key=api_key)

    # Call OCR with remote document URL
    ocr_response = client.ocr.process(
        model="mistral-ocr-latest",
        document={
            "type": "document_url",
            "document_url": args.url
        },
        include_image_base64=True
    )

    # Print OCR result
    print(ocr_response)

if __name__ == "__main__":
    main()

