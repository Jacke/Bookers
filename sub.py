import sys
import argparse
from youtube_transcript_api import YouTubeTranscriptApi

def extract_video_id(url_or_id):
    if "watch?v=" in url_or_id:
        return url_or_id.split("watch?v=")[1].split("&")[0]
    elif "youtu.be/" in url_or_id:
        return url_or_id.split("youtu.be/")[1].split("?")[0]
    else:
        return url_or_id

def fetch_transcript(video_id, languages):
    try:
        transcript = YouTubeTranscriptApi.get_transcript(video_id, languages=languages)
        return transcript
    except Exception as e:
        print(f"Ошибка при получении транскрипта: {e}")
        sys.exit(1)

def seconds_to_timestamp(seconds):
    hours = int(seconds // 3600)
    minutes = int((seconds % 3600) // 60)
    secs = int(seconds % 60)
    millis = int((seconds - int(seconds)) * 1000)
    return f"{hours:02}:{minutes:02}:{secs:02},{millis:03}"

def format_transcript(transcript, format_type):
    if format_type == 'txt':
        return "\n".join(entry['text'] for entry in transcript)

    elif format_type == 'srt':
        srt_lines = []
        for idx, entry in enumerate(transcript, start=1):
            start = seconds_to_timestamp(entry['start'])
            end = seconds_to_timestamp(entry['start'] + entry['duration'])
            text = entry['text']
            srt_lines.append(f"{idx}\n{start} --> {end}\n{text}\n")
        return "\n".join(srt_lines)

    elif format_type == 'raw':
        return transcript

    else:
        print(f"Неподдерживаемый формат: {format_type}")
        sys.exit(1)

def main():
    parser = argparse.ArgumentParser(description='Получение транскрипта YouTube-видео в различных форматах.')
    parser.add_argument('video', help='URL или идентификатор YouTube-видео')
    parser.add_argument('-f', '--format', choices=['txt', 'srt', 'raw'], default='txt', help='Формат вывода транскрипта')
    parser.add_argument('-l', '--languages', nargs='+', default=['en'], help='Предпочитаемые языки транскрипта (например, en ru)')
    args = parser.parse_args()

    video_id = extract_video_id(args.video)
    transcript = fetch_transcript(video_id, args.languages)

    if args.format == 'raw':
        for entry in transcript:
            print(entry)
    else:
        formatted_transcript = format_transcript(transcript, args.format)
        print(formatted_transcript)

if __name__ == "__main__":
    main()

