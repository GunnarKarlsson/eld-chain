# Runtime image: binary and deps come from the `eld-base` compile image
# (`deploy/scripts/build-docker-image-eld-node` builds that first, then this).
# Config and `wallets.json` are supplied by compose volume mounts.
#
#   docker build --build-context eld_base=docker-image://eld-base:<NODE_APP_VERSION_TAG> \
#     -f deploy/docker/Dockerfile.app .
FROM eld_base

RUN groupadd -r -g 1000 elduser && \
    useradd -r -u 1000 -g elduser elduser && \
    mkdir -p /app/data /app/config && \
    chown -R elduser:elduser /app/data /app/config

USER elduser

EXPOSE 26658
EXPOSE 8000
EXPOSE 6881
EXPOSE 25401
EXPOSE 8085
EXPOSE 4001
EXPOSE 4002

CMD ["./target/debug/eld-node"]
