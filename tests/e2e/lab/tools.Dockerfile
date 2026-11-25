FROM docker.io/library/alpine:3.24
RUN apk add --no-cache bash coreutils jq minisign tar gzip zip unzip
WORKDIR /work
