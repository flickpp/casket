FROM rust:1.64-buster AS BUILDER

# Python
RUN apt update -y && apt upgrade -y
RUN apt install -y python3.7-dev

WORKDIR /usr/src/casket
COPY src src
COPY Cargo.toml Cargo.toml
COPY Cargo.lock Cargo.lock
RUN cargo install --path .

FROM python:3.7-buster
COPY --from=BUILDER /usr/local/cargo/bin/casket /usr/local/bin/casket
EXPOSE 8080
ENV CASKET_RETURN_STACKTRACE_IN_BODY 0