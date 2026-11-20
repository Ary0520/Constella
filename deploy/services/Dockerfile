# Constella services in ONE container (keeps hosting cost minimal): relayer,
# keeper, watchdog, thesis API and the ECB feeder, behind one HTTP port.
# Build from the repo root:  docker build -f deploy/services/Dockerfile .
FROM python:3.12-slim
ENV PYTHONUNBUFFERED=1 PIP_NO_CACHE_DIR=1 PIP_ROOT_USER_ACTION=ignore
WORKDIR /app
COPY services/ /app/services/
RUN pip install -q /app/services/relayer /app/services/keeper /app/services/watchdog /app/services/feeder \
 && pip install -q -e /app/services/ai-engine
COPY deploy/services/supervisor.py deploy/services/proxy.py /app/
RUN mkdir -p /data
EXPOSE 8080
CMD ["python", "/app/supervisor.py"]
