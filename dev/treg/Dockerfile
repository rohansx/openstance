# Local, dev-only treg registry for openstance enrichment tests. Not for production.
FROM python:3.12-slim-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends build-essential && rm -rf /var/lib/apt/lists/* && pip install --no-cache-dir "uv>=0.12"
WORKDIR /app
COPY src/ /app/
RUN uv sync --frozen --extra server --no-dev
ENV TREG_EMAIL_DEV_MODE=true \
    TREG_FRONTEND_DEV=true \
    TREG_PUBLIC_URL=http://localhost:18790 \
    TREG_DATABASE_URL=sqlite+aiosqlite:////data/treg-dev.db
VOLUME /data
EXPOSE 18790
CMD ["sh", "-c", "uv run --no-sync python -m treg upgrade && uv run --no-sync python -m treg"]
