import time
import argparse
from selenium import webdriver
from selenium.webdriver.chrome.options import Options
from selenium.webdriver.common.by import By
from selenium.webdriver.support.ui import WebDriverWait
from selenium.webdriver.support import expected_conditions as EC

def download_txt_subtitle(youtube_url):
    options = Options()
#    options.add_argument("--headless=new")
    options.add_argument("--disable-gpu")
    options.add_argument("--no-sandbox")
    options.add_argument("--window-size=20,80")

    driver = webdriver.Chrome(options=options)
    driver.get("https://downsub.com/?url=" + youtube_url)

    try:
        # Ждём появления кнопки "TXT"
        txt_button = WebDriverWait(driver, 60).until(
            EC.element_to_be_clickable((By.XPATH, "//button[contains(., 'TXT')]"))
        )
        txt_button.click()
        print("Кнопка 'TXT' нажата.")

        # Ждём начала загрузки файла
        time.sleep(5)  # Подождите, пока загрузка начнётся

    except Exception as e:
        print(f"Произошла ошибка: {e}")
    finally:
        driver.quit()

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Скачивание .txt субтитров с YouTube через DownSub")
    parser.add_argument("url", help="Ссылка на YouTube-видео")
    args = parser.parse_args()

    download_txt_subtitle(args.url)

