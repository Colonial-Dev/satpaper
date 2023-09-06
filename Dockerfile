FROM rust:latest

WORKDIR /code
COPY . .

RUN cargo install --path .

ENTRYPOINT ["satpaper"]