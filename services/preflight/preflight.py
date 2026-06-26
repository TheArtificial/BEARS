#!/usr/bin/env python3
"""One-shot env validation for the BEARS compose stack (URI syntax + required secrets).

Runtime aggregation of similar checks (plus live DB/HTTP probes) is exposed on Den as
``GET /status`` and ``GET /status.json`` when the web server is enabled.
"""

from __future__ import annotations

import json
import os
import socket
import sys
import time
from pathlib import Path
from urllib.parse import urlparse

CURRENT_MODE = "startup"


def emit(stream, msg: str) -> None:
    print(msg, file=stream, flush=True)


def err(msg: str) -> None:
    emit(sys.stderr, f"preflight[{CURRENT_MODE}]: ERROR: {msg}")


def warn(msg: str) -> None:
    emit(sys.stderr, f"preflight[{CURRENT_MODE}]: WARNING: {msg}")


def info(msg: str) -> None:
    emit(sys.stderr, f"preflight[{CURRENT_MODE}]: {msg}")


def banner(title: str, msg: str) -> str:
    return "\n".join(
        [
            "",
            f"================ BEARS PREFLIGHT {title} [{CURRENT_MODE}] ================",
            msg,
            "===============================================================",
            "",
        ]
    )


def fail(msg: str) -> None:
    rendered = banner("FAILED", msg)
    emit(sys.stderr, rendered)
    emit(sys.stdout, rendered)
    sys.exit(1)


def success(msg: str) -> None:
    rendered = banner("OK", msg)
    emit(sys.stderr, rendered)
    emit(sys.stdout, rendered)


def require_non_empty(name: str) -> str:
    raw = os.environ.get(name)
    value = "" if raw is None else str(raw).strip()
    if not value or value == "SETME":
        fail(f"{name} must be set (current value is {value or 'empty'})")
    return value


def require_min_length(name: str, min_length: int) -> str:
    value = require_non_empty(name)
    if len(value) < min_length:
        fail(f"{name} must be at least {min_length} characters")
    return value


def parse_sql_uri(name: str, value: str) -> None:
    u = urlparse(value)
    if u.scheme not in ("postgres", "postgresql"):
        fail(f"{name} must use postgres:// or postgresql:// (got scheme {u.scheme!r})")
    if not u.hostname:
        fail(f"{name} must include a host name")


def redacted_sql_uri(value: str) -> str:
    u = urlparse(value)
    if not u.netloc:
        return "<unparseable>"
    auth = ""
    if u.username:
        auth = u.username
        if u.password:
            auth += ":***"
        auth += "@"
    host = u.hostname or ""
    if ":" in host and not host.startswith("["):
        host = f"[{host}]"
    port = f":{u.port}" if u.port else ""
    path = u.path or ""
    query = f"?{u.query}" if u.query else ""
    return f"{u.scheme}://{auth}{host}{port}{path}{query}"


def validate_sql_tcp_reachable(name: str, value: str, hint: str) -> None:
    u = urlparse(value)
    host = u.hostname
    port = u.port or 5432
    if not host:
        fail(f"{name} must include a host name")

    timeout_secs = float(os.environ.get("PREFLIGHT_DB_CONNECT_TIMEOUT_SECS", "3"))
    retries = int(os.environ.get("PREFLIGHT_DB_CONNECT_RETRIES", "5"))
    last_error = None

    info(
        f"{name} target {redacted_sql_uri(value)} "
        f"(host={host}, port={port}, connect_timeout={timeout_secs}s, retries={retries})"
    )

    try:
        addrs = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
        rendered = sorted(
            {
                f"{family.name if hasattr(family, 'name') else family}:{addr[0]}:{addr[1]}"
                for family, _, _, _, addr in addrs
            }
        )
        info(f"{name} DNS resolved {host} -> {', '.join(rendered)}")
    except OSError as exc:
        warn(f"{name} DNS lookup failed for {host}: {exc}")

    for attempt in range(1, retries + 1):
        try:
            info(f"{name} TCP connect attempt {attempt}/{retries} to {host}:{port}")
            with socket.create_connection((host, port), timeout=timeout_secs):
                info(f"{name} TCP reachable ({host}:{port})")
                return
        except OSError as exc:
            last_error = exc
            warn(f"{name} TCP connect attempt {attempt}/{retries} failed: {exc}")
            if attempt < retries:
                time.sleep(1)

    fail(
        f"{name} host is not reachable at {host}:{port} after {retries} attempts: {last_error}. "
        f"{hint}"
    )


def validate_http_url(name: str, value: str) -> None:
    u = urlparse(value.strip())
    if u.scheme not in ("http", "https"):
        fail(f"{name} must be an http(s) URL (got scheme {u.scheme!r})")
    if not u.netloc:
        fail(f"{name} must include a host (netloc)")


def validate_bifrost_model_metadata_config() -> None:
    path = Path(os.environ.get("BIFROST_CONFIG_PATH", "/app/bifrost/config.json"))
    if not path.exists():
        fail(
            f"BIFROST_CONFIG_PATH does not exist: {path}. The preflight image should bake services/bifrost/config.json from the Git-tracked build context."
        )

    try:
        config = json.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        fail(f"BIFROST_CONFIG_PATH is not valid JSON: {exc}")

    bears = config.get("bears")
    models = []
    if bears is None:
        warn(
            "Bifrost config has no top-level bears model metadata; Den will rely on the live /v1/models catalog"
        )
    elif not isinstance(bears, dict):
        fail("Bifrost config top-level bears value must be an object when present")
    else:
        models = bears.get("models")
        if not isinstance(models, list):
            fail(
                "Bifrost config bears.models must be an array when bears metadata is present"
            )

    if "auth_config" in config:
        fail(
            "Bifrost config top-level auth_config is deprecated; use governance.auth_config so /api/session/login is enabled after config-store reconciliation"
        )

    governance = config.get("governance")
    if not isinstance(governance, dict):
        fail(
            "Bifrost config must include governance.auth_config for runtime virtual-key provisioning"
        )
    auth_config = governance.get("auth_config")
    if not isinstance(auth_config, dict):
        fail(
            "Bifrost config must include governance.auth_config for runtime virtual-key provisioning"
        )
    if auth_config.get("is_enabled") is not True:
        fail(
            "Bifrost config governance.auth_config.is_enabled must be true for Den to provision virtual keys"
        )
    if auth_config.get("admin_username") != "env.BIFROST_ADMIN_USERNAME":
        fail(
            "Bifrost config governance.auth_config.admin_username must be env.BIFROST_ADMIN_USERNAME"
        )
    if auth_config.get("admin_password") != "env.BIFROST_ADMIN_PASSWORD":
        fail(
            "Bifrost config governance.auth_config.admin_password must be env.BIFROST_ADMIN_PASSWORD"
        )

    providers = config.get("providers")
    if not isinstance(providers, dict) or not providers:
        fail("Bifrost config providers must be a non-empty object")

    available_by_provider: dict[str, set[str]] = {}
    wildcard_providers: set[str] = set()
    for provider_name, provider in providers.items():
        if not isinstance(provider, dict):
            continue
        for key in provider.get("keys", []) or []:
            if not isinstance(key, dict):
                continue
            key_name = str(key.get("name", "<unnamed>")).strip() or "<unnamed>"
            key_models = key.get("models")
            if not isinstance(key_models, list) or not key_models:
                fail(
                    f"providers.{provider_name}.keys[{key_name}] must declare a non-empty models array; use ['*'] for provider-wide routing"
                )
            for model in key_models:
                if model == "*":
                    wildcard_providers.add(provider_name)
                elif isinstance(model, str) and model.strip():
                    available_by_provider.setdefault(provider_name, set()).add(
                        model.strip()
                    )

    seen_handles: set[str] = set()
    enabled_count = 0
    for idx, model in enumerate(models):
        if not isinstance(model, dict):
            fail(f"bears.models[{idx}] must be an object")
        if model.get("enabled", True):
            enabled_count += 1

        handle = str(model.get("handle", "")).strip()
        provider = str(model.get("provider", "")).strip()
        upstream_model = str(model.get("model", "")).strip()
        if not handle:
            fail(f"bears.models[{idx}].handle is required")
        if handle in seen_handles:
            fail(f"duplicate Bifrost model handle in bears.models: {handle}")
        seen_handles.add(handle)
        if not provider:
            fail(f"bears.models[{idx}].provider is required")
        if not upstream_model:
            fail(f"bears.models[{idx}].model is required")
        if provider not in providers:
            fail(f"bears.models[{idx}] references unknown provider {provider!r}")

        context_window = model.get("context_window")
        if not isinstance(context_window, int) or context_window < 1024:
            fail(f"bears.models[{idx}].context_window must be an integer >= 1024")
        max_output_tokens = model.get("max_output_tokens")
        if not isinstance(max_output_tokens, int) or max_output_tokens < 1:
            fail(f"bears.models[{idx}].max_output_tokens must be a positive integer")
        if max_output_tokens >= context_window:
            fail(
                f"bears.models[{idx}] max_output_tokens must be smaller than context_window ({handle})"
            )

        if (
            provider not in wildcard_providers
            and upstream_model not in available_by_provider.get(provider, set())
        ):
            fail(
                f"bears.models[{idx}] maps handle {handle!r} to {provider}/{upstream_model}, but that model is not listed under providers.{provider}.keys[].models"
            )

    if models and enabled_count == 0:
        fail("Bifrost config bears.models has no enabled models")
    if wildcard_providers:
        warn(
            "Bifrost provider keys use provider-wide routing (models: ['*']); "
            "explicit provider model lists are recommended only when you want preflight to validate exact availability"
        )

    if models:
        info(f"Bifrost model metadata OK ({enabled_count} enabled models in {path})")
    else:
        info(f"Bifrost provider config OK (no bears model metadata in {path})")


def validate_database_url(reachable: bool = True) -> None:
    database_url = require_non_empty("DATABASE_URL")
    parse_sql_uri("DATABASE_URL", database_url)
    info("DATABASE_URL parses as PostgreSQL URI")
    if reachable:
        validate_sql_tcp_reachable(
            "DATABASE_URL",
            database_url,
            "If you want the compose-bundled Postgres, enable COMPOSE_PROFILES=bundled; otherwise set DATABASE_URL to your managed Postgres.",
        )


def validate_config_shape() -> None:
    info("checking required secrets and URI-shaped environment variables")

    require_non_empty("JWT_SECRET")
    info("JWT_SECRET is set")

    require_min_length("DEN_SECRET_ENCRYPTION_KEY", 16)
    info("DEN_SECRET_ENCRYPTION_KEY is set")

    require_min_length("BIFROST_ENCRYPTION_KEY", 16)
    info("BIFROST_ENCRYPTION_KEY is set")
    require_non_empty("BIFROST_ADMIN_USERNAME")
    info("BIFROST_ADMIN_USERNAME is set")
    require_min_length("BIFROST_ADMIN_PASSWORD", 8)
    info("BIFROST_ADMIN_PASSWORD is set")

    validate_database_url(reachable=False)

    management = (
        os.environ.get("BIFROST_MANAGEMENT_URL", "").strip()
        or "http://bears-bifrost:8080/api"
    )
    validate_http_url("BIFROST_MANAGEMENT_URL", management)
    info(f"BIFROST_MANAGEMENT_URL OK ({management})")

    llm = os.environ.get("LLM_API_URL", "").strip() or "http://bears-bifrost:8080/v1"
    validate_http_url("LLM_API_URL", llm)
    info(f"LLM_API_URL OK ({llm})")

    web = require_non_empty("WEB_SERVER_URL")
    validate_http_url("WEB_SERVER_URL", web)
    info(f"WEB_SERVER_URL OK ({web})")

    require_non_empty("OPENAI_API_KEY")
    info("OPENAI_API_KEY is set")

    validate_bifrost_model_metadata_config()

    info("configuration shape checks passed")


def main() -> None:
    global CURRENT_MODE
    mode = sys.argv[1] if len(sys.argv) > 1 else "all"
    CURRENT_MODE = mode

    if mode == "config":
        validate_config_shape()
        success("configuration shape checks passed")
    elif mode == "den-db":
        validate_database_url(reachable=True)
        success("Den database reachability checks passed")
    elif mode == "all":
        validate_config_shape()
        validate_database_url(reachable=True)
        success("all preflight checks passed")
    else:
        fail(f"unknown preflight mode {mode!r}; expected config, den-db, or all")


if __name__ == "__main__":
    main()
