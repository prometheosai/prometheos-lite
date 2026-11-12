#!/usr/bin/env python3
"""Deterministic HTTPS fixture for the governed remote-service harness
(issue #154, Slice I).

A REAL TLS endpoint on loopback: the test suite spawns one fixture process
per test case and any HTTPS request it makes crosses an actual TLS handshake
with a certificate chain anchored in the embedded test CA. It is NOT a real
remote provider and proves transport behavior only.

Behavior is selected by environment variables:
  REMOTE_SERVICE_FIXTURE_MODE     ok | ok-no-usage | ok-json | model-mismatch |
                                  missing-model | malformed | oversize |
                                  redirect | status-500 | slow | secret-echo
                                  | require-auth
  REMOTE_SERVICE_FIXTURE_CERT     server (default) | wronghost
  REMOTE_SERVICE_FIXTURE_COUNT_FILE  when set, each request appends a line
  REMOTE_SERVICE_FIXTURE_API_KEY  secret for secret-echo / require-auth
  REMOTE_SERVICE_FIXTURE_EXPECT_AUTH  expected "Bearer <key>" for require-auth
  REMOTE_SERVICE_FIXTURE_REQUEST_LOG  when set, each POST body is appended as
                                      one line of JSON for wire assertions

Prints `PORT=<port>` on stdout once listening; runs until killed by the
parent test process.
"""

import json
import os
import ssl
import sys
import tempfile
import time
from http.server import BaseHTTPRequestHandler, HTTPServer

CA_PEM = """-----BEGIN CERTIFICATE-----
MIIDKzCCAhOgAwIBAgIUBzrrNAfQkeFQRdobx9aOAebXloswDQYJKoZIhvcNAQEL
BQAwJTEjMCEGA1UEAwwaUHJvbWV0aGVPUyBTbGljZS1JIFRlc3QgQ0EwHhcNMjYx
MTEyMTUyMjE1WhcNMzYxMTA5MTUyMjE1WjAlMSMwIQYDVQQDDBpQcm9tZXRoZU9T
IFNsaWNlLUkgVGVzdCBDQTCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEB
ANM9FIxh9AHOhVIG6fNR40b8+3yrr3uN9xNPckwDPZdGjsW2U+/pil9dSSss5ru7
1gNwVedTSPIIellJlarKW1jOoCjIyXhKQy/TGYx74VTqIJLg8YQfMHDKQdWSn+bX
4eCTDDyAZ7Wu3FBE2NzNEeLs4iohdiD8PY4vG78wKkN624g9ZlS3CwhfPbNoGZVD
ZWRJLHCyRS+IWNuKkCZBtQI+JUE+etgg1xbQE7jpSEjpLECrSgpkCQwG3MqYZewV
ajyr8Zw4GMWGQoWhW9q+bELRYvjKkiwGXeZ/8WEip79DVZUwR7Pz5yh+CAZvV05/
YmaOdCGVF0wvCx+OhDJKfOsCAwEAAaNTMFEwHQYDVR0OBBYEFDVh5WIUAti/OILo
UdH6djF2VY8UMB8GA1UdIwQYMBaAFDVh5WIUAti/OILoUdH6djF2VY8UMA8GA1Ud
EwEB/wQFMAMBAf8wDQYJKoZIhvcNAQELBQADggEBAL0DL6pINa3CE7gVYCkqfVwR
mWJT9hUWEHlNaGso+3dMX6oYp1WriG0xDejn8oGTZ1hKhZ4KER8oRSZU+IK+Wjy2
K7WXESPowxCbtbYNOXCfjMmubAQt9FtdpUAy5YutxjqzpDVyCRbFWgEFfJh7QQKP
71+0UDkf8V7TPP+mw6W4vUAf52lNbjDKvK+pn9QErMxcNql8LpNcTMS3jbaP7gZJ
eBpkW7NcRGVHmoeeqZrpERhfJn5mtAKPVTzrk2UZQ7MOTPdO23jyCTY6DjD24OPn
nSdKs47cVOd7jkTF/a9wDixA3r1EBb1GfmfqFuwQrBaZVUJqIAJVkbWaw0TL3GU=
-----END CERTIFICATE-----
"""

SERVER_CERT = """-----BEGIN CERTIFICATE-----
MIIDWjCCAkKgAwIBAgIUMl6YWX4unRma7mBs7OnDrSN2JI4wDQYJKoZIhvcNAQEL
BQAwJTEjMCEGA1UEAwwaUHJvbWV0aGVPUyBTbGljZS1JIFRlc3QgQ0EwHhcNMjYx
MTEyMTUyMjE2WhcNMzYxMTA5MTUyMjE2WjAUMRIwEAYDVQQDDAlsb2NhbGhvc3Qw
ggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQC5vcGLHa4EnN4vKonKtceQ
DCxwxw21i2TKyCUVNG5U/HGwYjcVTXoDPKK/xPWvf2Dfmi70RyadoS5drjY8rh6I
7dj0mPA5nnl+wH6BnUXsrYOewfc1iGzTSjA+JmpQP6V/8M2L1xZsMa0OXKJs6Kuf
s7mnkOy/eyoFDavArlndKgeySpaVKpvA3CVAJHCPFCuhT9mbZ6VREWykImq5ZmCv
3uLafiLjfA2GRGl2dpjub5Clz3vgRC80DzWLa9JGoKgUj68GkFvjya7pmUi7ATj0
qKvI8sCJf7HKBtDyxYSgq3Ovy2GUX1PS19mJhn8Jj15gRFuNd6Ik96DewmEX51vn
AgMBAAGjgZIwgY8wDAYDVR0TAQH/BAIwADAOBgNVHQ8BAf8EBAMCBaAwEwYDVR0l
BAwwCgYIKwYBBQUHAwEwGgYDVR0RBBMwEYIJbG9jYWxob3N0hwR/AAABMB0GA1Ud
DgQWBBRohWEKyB+xG6PQCLswMo1pCHVyCjAfBgNVHSMEGDAWgBQ1YeViFALYvziC
6FHR+nYxdlWPFDANBgkqhkiG9w0BAQsFAAOCAQEAJ4Jegew+Pb6wp5I8oP0oHzGZ
dw+/V83hg8MVzfDa5dtVgEjtt2mCCoKqn9ttqgpTteLbgyzKB0LmYg100TrrkQsE
6bV3Kbfru9hq3Hf/nuQG8K+fGqAOQhVvs4qFXzkVbwmy40TdlbokXT0kLShOhgpX
8csVRJNeAKIfpiIygMqpGENNtHROk8dhpxGlS7Qbx9OzzfoLBKCNfuVy5CbF6aw+
gh0uayuVF4ekyLMylNPOarGaAUn37Kk+e1B84BofmN+Y7P5ZsrOM0poM4a183c9P
sNu8b9TRFRHjJjNdxWTvq4nZThd1AvB50aM6U3/5zAlO9uuuyZq/J4TB/r+8uQ==
-----END CERTIFICATE-----
"""

SERVER_KEY = """-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQC5vcGLHa4EnN4v
KonKtceQDCxwxw21i2TKyCUVNG5U/HGwYjcVTXoDPKK/xPWvf2Dfmi70RyadoS5d
rjY8rh6I7dj0mPA5nnl+wH6BnUXsrYOewfc1iGzTSjA+JmpQP6V/8M2L1xZsMa0O
XKJs6Kufs7mnkOy/eyoFDavArlndKgeySpaVKpvA3CVAJHCPFCuhT9mbZ6VREWyk
Imq5ZmCv3uLafiLjfA2GRGl2dpjub5Clz3vgRC80DzWLa9JGoKgUj68GkFvjya7p
mUi7ATj0qKvI8sCJf7HKBtDyxYSgq3Ovy2GUX1PS19mJhn8Jj15gRFuNd6Ik96De
wmEX51vnAgMBAAECggEAJe2MJIKygqmfqyT2Dykwb4c02fCjqYMl2UymWd6jewIz
mu9rTRPNivSqTRuEicoqd25uER6oUX7Uz3dgBuMNRCvBZi0GvNVR/OKE0qXRXgOO
gXAdMVbC1iBrmn1Qr0NX4bjD2D7bCsvYCTs8RZ37Ubiq+/8ZgP4+9do1TLqtif4O
T+Y9TQjTYQhwK73+mPwskbwoJkDGFfxpCgr60GnuEzqhvCne6+bvs0V/2BHGlFBK
S4osZZPBrtwdIfTenoBVWVYyVuzfKJm9dN3kHGxzcNvIY2SlSC8OsHhW8zdcFoIK
05WSfwDJa76f+PqvUTaB/zFXYpU7tg53hy7fgsLMfQKBgQDr+oSNALmkvUOe70ym
fS+rlDO2TdtAtkX3gNqclABnMkSLkqWg12HVFiOJWtF7tvbDSFGkDxeeYKkrn/Cu
L36fYAjQDeHPIRBq9iAUGnbkgLh4ualjOzpEGogZ9lld8OxwG3ScGq6Uxo4Zavjr
RypXdNL86Z2SdV9McThcbRemLQKBgQDJgBPBr6QUmbe9c/iUjj0KNLaQC8rYHOnV
rO+LbyNCw1No2t9r8pE7gRocZxlNui+cXkutX6NHNqCCEhWP5ZuQ+u6T9mROiufm
50Ctro+hn/D6ok4uhyVq8J7kXp4ciAMDR9BAYXUhemCTy12V08livzCf4ey09Xsq
UjIJwz5K4wKBgHC+sLXtssTS6Iv3KVgiV7RSiqJ/cFHqxGMMXWaSD98+VSGaXygJ
v+RfP3tQuOWy9RM4aklILLCu3iTJteSDLKxY5d0BxPGBiqaQEaOJQWTLW1fS3Otc
UOpZAs3kta38ffrSI1SyKtdKRno+cc93oPNIDcZol7ByA6YyMgt20+XBAoGBAKvI
xQ8a7oehhUO+2u3LVb6dsgacBxK3k6LXDGGTHjyMyU9bSZ597dMM/XW5b5kUYgKt
YVctccYEjFJXmIpU9YtkYoFCTcGoKoxwGnP3nfvsn0UNmhmQnhXNjaikAafRTMA/
OQuqCnHAIxXdCqvxIn9M/+dOdL7RiW9LI92u1iyBAoGBAMH1K0clEL9DAGev2WJy
RWzqYTtzGSUuTw8EvhyWGfR1BY8O+7AZxfdIBO4ARKHldJCkcEfKPB1FlIzZQ2Cm
YEwljd/W5EWzdSu6PhY8A1DGhbmU4yFOOnwwhZ+/UxfmPECUFHCOO3A79CKZp/rq
agUcItmg6XyFo4iBdwKgH113
-----END PRIVATE KEY-----
"""

WRONGHOST_CERT = """-----BEGIN CERTIFICATE-----
MIIDdDCCAlygAwIBAgIUMl6YWX4unRma7mBs7OnDrSN2JI8wDQYJKoZIhvcNAQEL
BQAwJTEjMCEGA1UEAwwaUHJvbWV0aGVPUyBTbGljZS1JIFRlc3QgQ0EwHhcNMjYx
MTEyMTUyMjE2WhcNMzYxMTA5MTUyMjE2WjAkMSIwIAYDVQQDDBl3cm9uZ2hvc3Qt
Zml4dHVyZS5pbnZhbGlkMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA
4DJlV5nIfJXYcdFGhhJhmo0jj7dC/5kHouKDhVis6on5TWqVpR5pbBKEOzJ/qVFS
Pot7u2l9jI1PZSsDlAFW4QJPDuz7054l203QA3IKfNs5ZOvuJJbik/VGIas1tWN8
DakBhcpOBE2vsx3ffoSV1qf05IE3dEpVdOkROswl3CS4IyOgBo2CIIdLR8TMPDDC
CN9UL5N5PvvYP2sZ9d7D13yZAnIvyFOq6DwWDBdty1Gj2fU/AUej+5VLt4k6Ft5P
L0f8jqqnLGv1m8cjElzqyidBIPvWQbknMo50QxM1S0oEfzbni9a9RHB9r3ycTT+P
vQnUG9QiY2iMU/7eRxZI6QIDAQABo4GcMIGZMAwGA1UdEwEB/wQCMAAwDgYDVR0P
AQH/BAQDAgWgMBMGA1UdJQQMMAoGCCsGAQUFBwMBMCQGA1UdEQQdMBuCGXdyb25n
aG9zdC1maXh0dXJlLmludmFsaWQwHQYDVR0OBBYEFEmCL2xaDBtswYsOt0TY/fjM
IWwgMB8GA1UdIwQYMBaAFDVh5WIUAti/OILoUdH6djF2VY8UMA0GCSqGSIb3DQEB
CwUAA4IBAQBr91fvATXMQzRKUT6l4T/ibW454bjL+VxKyNejXcVVSV+I7O4p/Ilj
kQAWB8nPWfQfV3i4ZWp/9IfH0mj1nH4AR5UyZgUcg1FfMkGRTE1gNVvx8R3tbzO9
dxRtARP3eibTRMz+zDysihJFbbDECcDby+EKYibl1vkDGCfofvnrR0gxS96S58yw
vnNuiNzj/UOPDH5kTsQZ9+GVOQaNrP2vbZyGu1MSOUSdVSLkmJenHD+TXdAKbAZt
Rv/EkP6vUSLPD6t9T9S3KZp3RHoZWFUII06t32XKpGmGhQfJt3a1PTuu2c2hr4SD
u5/D9iq07jG+gaeeIzSK4XjeAGTMXx9M
-----END CERTIFICATE-----
"""

WRONGHOST_KEY = """-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDgMmVXmch8ldhx
0UaGEmGajSOPt0L/mQei4oOFWKzqiflNapWlHmlsEoQ7Mn+pUVI+i3u7aX2MjU9l
KwOUAVbhAk8O7PvTniXbTdADcgp82zlk6+4kluKT9UYhqzW1Y3wNqQGFyk4ETa+z
Hd9+hJXWp/TkgTd0SlV06RE6zCXcJLgjI6AGjYIgh0tHxMw8MMII31Qvk3k++9g/
axn13sPXfJkCci/IU6roPBYMF23LUaPZ9T8BR6P7lUu3iToW3k8vR/yOqqcsa/Wb
xyMSXOrKJ0Eg+9ZBuScyjnRDEzVLSgR/NueL1r1EcH2vfJxNP4+9CdQb1CJjaIxT
/t5HFkjpAgMBAAECggEAJ+/KPivSeyDtNL8o/yaElPWZ1Va5nFLL+a+8anNEoCXy
TyBSpl5vvJTi0bG57HUkP6h0osqUlwYwE68xHLsNWRkicJ/17vkfuLxYdtiodhvO
EkR1wDZRh7Y8JzC6qUReu4c00nqaYaleCDvkzUJ8d4elN9Qtxu0b4m7HBzCI+PkL
KBETlrOzOTrQBMT77gPDfIbbxVWWHk/khPfJKuT517MP74SdarqZcSEV/Aa2CqeX
F4L8btAwKJI4DRDBxWFKYq6ud42BO0myjld43FNLTiXs/yWKwvFCL1+fsCyRzsLp
R5w7NskHplOYgwB1iR3Z3qyeEhLAdlOkRTPCGuIMtQKBgQDzgY/m0QOi1CGBMEiV
39N+SNJ4PVdWR2ZBNqtNNRGzhgKGHvM9vi8umF/1xa2x5YN7jwPdxYLoSKzOx9mj
xmOT0H6wMtZKIYGB0dktuEqIxzVr1envwtvj7QaPewtCqmerQCxejjNp6TC2BHCw
UPMZpiGIRpqEvlrxPOqmeQxshQKBgQDrszVAa5d7CAudEg1lsv6X5nvHSm3tMH/x
1SZmmaqSAXNT6SLKvGv1SBSAhGlvu0s3XzVs7QJeoNBznEHawvIEh7hWtZhLdd/I
L4YtRjN7MTmzdLCHRsPYQGuQ6K1VbQ59cF6KvqSJ4Sw2VCZOfLcOeIXadTWGfIV1
v8k7XBJ6FQKBgQCo5df7QESmXz7yDZx7ICng8P/TWLDIbnO1jgbZYZLqk2wSvUr4
K38scZI1taxZtRkyzarf20yGc0oxjxsdZtDQyQ/mhQxvlLfwZK6f8LD6GDqslk5R
+DVu7YvcX3sa/Xk0pilgWH++2Xjo90F2el90DfA6RsaMhkAsDZWwbEu6xQKBgEc/
xhIyM/liBji180zR3fpHFxW4XyB7L8IH3/LV8uTuAVHVQMouSSg9YM00uban1GXp
pvnFb5jxXfIEwzeBBK3v5P55KI9vE3WtQA9BkGBmEf44gQBzmNIvLxT/8R3IuL+c
j0e7t3tikP3fVDEXrDJ4pmXSVprP23wrvIXwIDm9AoGBAJhJXEGYHF99PY5zdWa+
eIo8iOmZR8V2qZ6FeAFy00wkE3WSZzTvHHQuAyRtSBPVnttHaLgPl+Vp0K6jl8a1
9zU3ne0ZRJ42etRNUFPUNdfEgeqgnwtE8q0iaTQ66iGNXyuQPME0L6/VWj8K/ujh
NNLgJxF8M7IL8TKz6ptvrTGw
-----END PRIVATE KEY-----
"""

MODE = os.environ.get("REMOTE_SERVICE_FIXTURE_MODE", "ok")
CERT_KIND = os.environ.get("REMOTE_SERVICE_FIXTURE_CERT", "server")
COUNT_FILE = os.environ.get("REMOTE_SERVICE_FIXTURE_COUNT_FILE", "")
EXPECT_AUTH = os.environ.get("REMOTE_SERVICE_FIXTURE_EXPECT_AUTH", "")
ECHO_KEY = os.environ.get("REMOTE_SERVICE_FIXTURE_API_KEY", "")
REQUEST_LOG = os.environ.get("REMOTE_SERVICE_FIXTURE_REQUEST_LOG", "")
MODEL = "tiny-remote-model"


def record_hit() -> None:
    if COUNT_FILE:
        try:
            with open(COUNT_FILE, "a", encoding="utf-8") as handle:
                handle.write("hit\n")
        except OSError:
            pass


def record_request(body: bytes) -> None:
    if REQUEST_LOG:
        try:
            with open(REQUEST_LOG, "a", encoding="utf-8") as handle:
                handle.write(body.decode("utf-8", "replace") + "\n")
        except OSError:
            pass


def usage_block():
    if MODE == "ok-no-usage":
        return None
    return {"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18}


def chat_response():
    if MODE == "model-mismatch":
        model = "other-model-3"
    elif MODE == "missing-model":
        model = None
    else:
        model = MODEL
    if MODE == "ok-json":
        content = json.dumps({"answer": 42})
    elif MODE == "secret-echo":
        content = ECHO_KEY + " fixture response"
    else:
        content = "fixture response"
    payload = {
        "id": "chatcmpl-fixture",
        "object": "chat.completion",
        "created": 1768000000,
        "model": model,
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content,
                },
                "finish_reason": "stop",
            }
        ],
    }
    usage = usage_block()
    if usage is not None:
        payload["usage"] = usage
    if MODE == "missing-model":
        payload.pop("model", None)
    if MODE == "oversize":
        payload["choices"][0]["message"]["content"] = "X" * 300000
    return json.dumps(payload).encode("utf-8")


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):  # keep the test log clean
        pass

    def _send(self, code: int, body: bytes, content_type: str = "application/json") -> None:
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self) -> None:
        record_hit()
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            length = 0
        body = self.rfile.read(length) if length > 0 else b""
        record_request(body)
        if MODE == "slow":
            time.sleep(10)
        if self.path == "/v1/chat/completions":
            if MODE == "redirect":
                self.send_response(301)
                self.send_header("Location", "https://127.0.0.1:1/health")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            if MODE == "require-auth":
                if self.headers.get("Authorization", "") != EXPECT_AUTH:
                    body = json.dumps({"error": {"message": "unauthorized"}}).encode()
                    self._send(401, body)
                    return
            if MODE == "status-500":
                body = json.dumps({"error": {"message": "provider exploded with fake-secret-key-12345"}}).encode()
                self._send(500, body)
                return
            if MODE == "malformed":
                self._send(200, b"not json{{{")
                return
            self._send(200, chat_response())
            return
        self._send(404, json.dumps({"error": "not found"}).encode())

    def do_GET(self) -> None:
        record_hit()
        if self.path == "/health":
            self._send(200, json.dumps({"status": "ok"}).encode())
            return
        if MODE == "redirect" and self.path in ("/v1/redirect", "/v1/chat/completions"):
            self.send_response(301)
            self.send_header("Location", "https://127.0.0.1:1/health")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        self._send(404, json.dumps({"error": "not found"}).encode())


def main() -> None:
    cert_pem, key_pem = (SERVER_CERT, SERVER_KEY) if CERT_KIND == "server" else (WRONGHOST_CERT, WRONGHOST_KEY)
    with tempfile.TemporaryDirectory() as tmp:
        cert_path = os.path.join(tmp, "fixture.crt")
        key_path = os.path.join(tmp, "fixture.key")
        with open(cert_path, "w", encoding="utf-8") as handle:
            handle.write(cert_pem)
        with open(key_path, "w", encoding="utf-8") as handle:
            handle.write(key_pem)
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(cert_path, key_path)
        server = HTTPServer(("127.0.0.1", 0), Handler)
        server.socket = ctx.wrap_socket(server.socket, server_side=True)
        print(f"PORT={server.server_address[1]}", flush=True)
        server.serve_forever()


if __name__ == "__main__":
    main()
