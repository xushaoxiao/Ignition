# Docker assets

Local development containers for Ignition.

| File | Purpose |
|---|---|
| [`compose.yml`](./compose.yml) | Postgres 16 + Redis 7 for local `make reset` / `make run` |

Future image Dockerfiles (API, TMA static) should land here next to Compose.

## Usage

From the repository root:

```bash
make up
make down
# or explicitly:
docker compose -f docker/compose.yml up -d --wait
```

Ports (host → container): Postgres `55432→5432`, Redis `56379→6379`.
