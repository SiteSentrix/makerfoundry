# MakerFoundry's web service as a container, for Railway or any container host. Build from the
# repository root:
#   docker build -t makerfoundry .
# Settings come from the environment (docs/deploy.md); nothing secret is built in.
FROM rust:1.97.0-bookworm AS build
# The image's own toolchain, the one rust-toolchain.toml names, so rustup downloads nothing.
ENV RUSTUP_TOOLCHAIN=1.97.0
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p makerfoundry

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home makerfoundry
COPY --from=build /src/target/release/makerfoundry /usr/local/bin/makerfoundry
USER makerfoundry
# Railway gives the port in PORT; 8080 when run elsewhere.
ENV PORT=8080
EXPOSE 8080
CMD ["makerfoundry"]
