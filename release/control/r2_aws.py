#!/usr/bin/env python3
"""Run AWS CLI with R2 S3 credentials injected by Infisical."""

import os
import sys


def main() -> None:
    access_key = os.environ.get("AWS_ACCESS_KEY_ID")
    secret_key = os.environ.get("AWS_SECRET_ACCESS_KEY")
    if not access_key or not secret_key:
        sys.exit("ATMX R2 credentials are missing in Infisical prod: set "
                 "AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY")
    # Infisical may inject unrelated production secrets. Do not pass those on
    # to the AWS subprocess; it only needs this bucket's scoped S3 pair.
    keep = ("PATH", "HOME", "TMPDIR", "LANG", "LC_ALL", "HTTPS_PROXY", "HTTP_PROXY",
            "NO_PROXY", "SSL_CERT_FILE", "AWS_CA_BUNDLE")
    environment = {key: os.environ[key] for key in keep if key in os.environ}
    environment["AWS_ACCESS_KEY_ID"] = access_key
    environment["AWS_SECRET_ACCESS_KEY"] = secret_key
    environment["AWS_DEFAULT_REGION"] = "auto"
    environment["AWS_REGION"] = "auto"
    environment["AWS_EC2_METADATA_DISABLED"] = "true"
    os.execvpe("aws", ["aws", *sys.argv[1:]], environment)


if __name__ == "__main__":
    main()
