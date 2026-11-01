FROM node:22-bookworm-slim AS node-runtime

# yt-dlp and its PO-token plugin are Python programs installed with pip, so the
# image keeps a Python base for them. The service around them is Node, the
# same runtime yt-dlp uses for YouTube's JavaScript challenges.
FROM python:3.13-slim-bookworm

ARG YTDLP_VERSION=2026.8.19
# The yt-dlp plugin installed here and the pot-provider container in compose
# are two halves of one protocol, so they move together. 2.0.0 fixes an RCE in
# the provider (GHSA-qpv9-8xfj-xx9m); its image still listens on 0.0.0.0
# inside the container, so the compose network keeps reaching it.
ARG POT_PROVIDER_VERSION=2.0.0

COPY --from=node-runtime /usr/local/bin/node /usr/local/bin/node

RUN python -m pip install --no-cache-dir \
    "yt-dlp[default]==${YTDLP_VERSION}" \
    "bgutil-ytdlp-pot-provider==${POT_PROVIDER_VERSION}" \
    && useradd --create-home --uid 10001 tawny

WORKDIR /app
COPY --chown=tawny:tawny server.mjs /app/server.mjs

USER tawny
ENV PORT=8080 \
    PYTHONDONTWRITEBYTECODE=1

EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=4s --start-period=10s --retries=6 \
    CMD ["node", "-e", "fetch('http://127.0.0.1:8080/health',{signal:AbortSignal.timeout(3000)}).then(r=>process.exit(r.ok?0:1),()=>process.exit(1))"]

ENTRYPOINT ["node", "/app/server.mjs"]
