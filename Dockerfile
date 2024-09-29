#== Second stage: 
FROM docker.io/library/ubuntu:20.04
LABEL description="EightFish:http_gate"

WORKDIR /eightfish

RUN mkdir -p /eightfish/target/wasm32-wasi/release/

COPY ./spin /usr/local/bin
COPY ./spin.toml /eightfish/http_gate_spin.toml
COPY ./target/wasm32-wasi/release/http_gate.wasm /eightfish/target/wasm32-wasi/release/
