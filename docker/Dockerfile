FROM rust:alpine as base

WORKDIR /code

COPY Cargo.toml .
COPY Cargo.lock .
COPY src/ src/

RUN apk update && apk add g++ zlib zlib-dev

RUN cargo fetch --locked

# Run tests
FROM base as test

RUN cargo test --release

# Build binary
FROM test as build

RUN cargo build --release

# Package image
FROM alpine as production

WORKDIR /home/rust/

RUN apk update && apk add supervisor

COPY docker/supervisord.conf /etc/supervisor/conf.d/supervisord.conf

COPY --from=build /code/target/release/satpaper .

CMD [ "/usr/bin/supervisord", "-c", "/etc/supervisor/conf.d/supervisord.conf" ]