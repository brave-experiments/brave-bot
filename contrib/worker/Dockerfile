# Optional worker image: bravebot + aws-cli + an S3 sync loop.
#
# Lives under contrib/: useful for headless Jobs, not part of the shipped CLI and
# not built by CI. Brave's internal EKS Job templates and ECR tags live in the
# deploying organisation's ops repo; this directory is the reusable image recipe.
#
# General-purpose agent toolkit (not task-specific): common CLI utilities, Node,
# a practical Python stack, and duckdb. Intentionally no torch/CUDA / full ML
# frameworks — add those in a derived image if a task needs them.
#
# Build from the repository root (must match the CLI image platform):
#
#   docker build --platform linux/amd64 -f contrib/worker/Dockerfile \
#     --build-arg BASE_IMAGE=bravebot:0.4.0 \
#     -t bravebot-worker:0.4.0 \
#     .
#
# Required env at run time: BUCKET, TASK_ID, TASK_PROMPT (see run.sh).

ARG BASE_IMAGE=bravebot:0.4.0
ARG AWS_ARCH=x86_64

FROM ${BASE_IMAGE} AS cli

FROM debian:bookworm-slim AS runtime
ARG AWS_ARCH

USER root
# General agent workspace tools. duckdb via pip (not in bookworm apt).
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
        curl \
        wget \
        unzip \
        zip \
        findutils \
        coreutils \
        diffutils \
        patch \
        procps \
        openssl \
        less \
        ripgrep \
        fd-find \
        git \
        jq \
        file \
        tree \
        moreutils \
        sqlite3 \
        pigz \
        xz-utils \
        bzip2 \
        build-essential \
        nodejs \
        npm \
        python3 \
        python3-pip \
        python3-venv \
        python3-dev \
        python3-numpy \
        python3-pandas \
        python3-scipy \
        python3-sklearn \
        python3-requests \
        python3-yaml \
        python3-bs4 \
        python3-lxml \
        python3-pil \
        python3-openpyxl \
    && curl -fsSL "https://awscli.amazonaws.com/awscli-exe-linux-${AWS_ARCH}.zip" -o /tmp/awscliv2.zip \
    && unzip -q /tmp/awscliv2.zip -d /tmp \
    && /tmp/aws/install \
    && rm -rf /tmp/aws /tmp/awscliv2.zip /var/lib/apt/lists/* \
    && ln -sf /usr/bin/fdfind /usr/local/bin/fd \
    && pip3 install --break-system-packages --no-cache-dir \
        'duckdb>=1.0,<2' \
        'jsonlines>=4,<5' \
        'httpx>=0.27,<1' \
        'rich>=13,<15' \
    && groupadd --gid 1000 bravebot \
    && useradd --uid 1000 --gid 1000 --create-home --home-dir /data bravebot \
    && mkdir -p /scripts /workspace /data/.bravebot \
    && chown -R bravebot:bravebot /workspace /data

COPY --from=cli /usr/local/bin/bravebot /usr/local/bin/bravebot
COPY contrib/worker/run.sh /scripts/run.sh
COPY contrib/worker/settings.json /scripts/settings.json
COPY contrib/worker/settings.json /data/.bravebot/settings.json

RUN chmod +x /scripts/run.sh /usr/local/bin/bravebot \
    && chown -R bravebot:bravebot /data/.bravebot /scripts

USER bravebot
ENV HOME=/data
WORKDIR /workspace

ENTRYPOINT ["/scripts/run.sh"]
