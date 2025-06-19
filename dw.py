from youtube_transcript_api import YouTubeTranscriptApi
from youtube_transcript_api.formatters import JSONFormatter

ytt_api = YouTubeTranscriptApi()
transcript = ytt_api.fetch("_YFWOTHUVZA")
formatter = JSONFormatter()

json_formatted = formatter.format_transcript(transcript)

with open('your_filename.json', 'w', encoding='utf-8') as json_file:
    json_file.write(json_formatted)


