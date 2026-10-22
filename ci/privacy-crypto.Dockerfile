FROM python:3.13-slim-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends age ca-certificates && rm -rf /var/lib/apt/lists/*
RUN useradd --uid 10001 --create-home drill
USER drill
CMD ["python3", "-c", "import time; time.sleep(3600)"]
