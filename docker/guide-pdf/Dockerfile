# Asciidoctor PDF and what scripts/guide-fonts.sh needs to make the User
# Guide's fonts, as CI builds the guide, so `make user-guide-pdf` needs only
# Docker. The versions match the guide job in .github/workflows/ci.yml.
FROM ruby:3.3.6-slim-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl fontconfig fonts-dejavu-core python3-fonttools \
    && rm -rf /var/lib/apt/lists/* \
    && gem install --no-document asciidoctor-pdf -v 2.3.15

WORKDIR /work
