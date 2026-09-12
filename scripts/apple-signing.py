#!/usr/bin/env python3
"""Configure CI signing without logging secrets or downgrading partial setups."""
import base64
import os
from pathlib import Path
import uuid

SIGNING = ("APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "APPLE_SIGNING_IDENTITY", "APPLE_TEAM_ID")
NOTARY = ("APPLE_API_KEY", "APPLE_API_ISSUER", "APPLE_API_KEY_BASE64")


def configure(source, env_file, output_file, temporary):
    present = [bool(source.get(key)) for key in SIGNING]
    notarize = [bool(source.get(key)) for key in NOTARY]
    if any(present) and not all(present):
        raise ValueError("Apple signing is incomplete. Configure all four signing secrets or remove the partial configuration.")
    if any(notarize) and (not all(notarize) or not all(present)):
        raise ValueError("Notarization requires all three API-key secrets and a complete Developer ID configuration.")
    values = {"APPLE_SIGNING_IDENTITY": "-"}
    state = "adhoc"
    if all(present):
        if source["APPLE_SIGNING_IDENTITY"].strip() == "-":
            raise ValueError("A configured Developer ID certificate cannot use the ad-hoc signing identity.")
        values = {key: source[key] for key in SIGNING}
        state = "developer-id"
    if all(notarize):
        try:
            key_bytes = base64.b64decode("".join(source["APPLE_API_KEY_BASE64"].split()), validate=True)
        except ValueError:
            raise ValueError("The notarization key is not valid base64.") from None
        if not key_bytes.startswith(b"-----BEGIN PRIVATE KEY-----"):
            raise ValueError("The notarization key must be a .p8 private key.")
        key_file = temporary / "anima-notary" / "AuthKey.p8"
        key_file.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(key_file, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(key_bytes)
        values.update({key: source[key] for key in NOTARY[:2]})
        values["APPLE_API_KEY_PATH"] = str(key_file)
        state = "notarized"
    with env_file.open("a", encoding="utf-8") as stream:
        for name, value in values.items():
            delimiter = "ANIMA_" + uuid.uuid4().hex
            stream.write(f"{name}<<{delimiter}\n{value}\n{delimiter}\n")
    with output_file.open("a", encoding="utf-8") as stream:
        stream.write(f"signing={state}\n")
    return state


if __name__ == "__main__":
    try:
        state = configure(os.environ, Path(os.environ["GITHUB_ENV"]),
                          Path(os.environ["GITHUB_OUTPUT"]), Path(os.environ["RUNNER_TEMP"]))
        print("Apple build configuration: " + state)
    except ValueError as error:
        raise SystemExit(str(error))
