# Local treg (dev only)

A self-hosted [treg](https://github.com/superdesigndev/treg) registry running in Docker/OrbStack.
It's used to test openstance's `treg` enricher without touching treg.to or spending money.

```sh
git clone --depth 1 https://github.com/superdesigndev/treg dev/treg/src   # source is gitignored
printf 'TREG_SECRET_KEY=%s\nTREG_SESSION_SECRET=%s\n' \
  "$(python3 -c 'import base64,os;print(base64.urlsafe_b64encode(os.urandom(32)).decode())')" \
  "$(openssl rand -hex 24)" > dev/treg/.env                               # gitignored
docker compose -f dev/treg/compose.yaml up -d --build
open http://127.0.0.1:18790   # email OTP dev mode: the sign-in code is shown on the page
```

Notes:
- The server binds to loopback only, and its data lives in the `treg-data` volume.
- A self-hosted registry has no provider keys of its own. Endpoints with a public route work for free; anything else needs your own provider key added as a team credential. Automated tests use `MockEnricher` and a mock provider instead, so CI never calls real providers.
- treg's licence allows self-hosting for your own use. Don't offer it to third parties as a hosted service.
