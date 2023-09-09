FROM rust:alpine as builder

WORKDIR /code

COPY Cargo.toml .
COPY Cargo.lock .
COPY src/ src/

RUN apk update && apk add g++ zlib zlib-dev

RUN cargo build --locked --release

ENTRYPOINT ["./target/release/satpaper"]

FROM alpine
WORKDIR /home/rust/

RUN apk update && apk add supervisor

COPY supervisord.conf /etc/supervisor/conf.d/supervisord.conf

COPY --from=builder /code/target/release/satpaper .

CMD [ "/usr/bin/supervisord", "-c", "/etc/supervisor/conf.d/supervisord.conf" ]