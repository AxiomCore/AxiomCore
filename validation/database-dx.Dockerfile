FROM postgres:17-bookworm AS pg17
FROM postgres:18-bookworm AS pg18
FROM rust:1.93-bookworm
RUN apt-get update -qq && apt-get install -y -qq libsqlite3-0 libssl-dev pkg-config clang python3 libicu72 libldap-2.5-0 liblz4-1 libzstd1 liburing2 && rm -rf /var/lib/apt/lists/*
COPY --from=pg17 /usr/lib/postgresql/17 /usr/lib/postgresql/17
COPY --from=pg17 /usr/share/postgresql/17 /usr/share/postgresql/17
COPY --from=pg18 /usr/lib/postgresql/18 /usr/lib/postgresql/18
COPY --from=pg18 /usr/share/postgresql/18 /usr/share/postgresql/18
RUN rm /usr/share/postgresql/17/postgresql.conf.sample /usr/share/postgresql/18/postgresql.conf.sample
COPY --from=pg17 /usr/share/postgresql/postgresql.conf.sample /usr/share/postgresql/17/postgresql.conf.sample
COPY --from=pg18 /usr/share/postgresql/postgresql.conf.sample /usr/share/postgresql/18/postgresql.conf.sample
RUN useradd -m axiom
WORKDIR /workspace
ENV CARGO_TARGET_DIR=/work/database-dx-linux-target CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
