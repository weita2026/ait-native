from __future__ import annotations

import base64
import hashlib
import hmac
import json
import secrets
from datetime import datetime, timedelta, timezone
from typing import Any

from ait_protocol.common import generate_workflow_id, normalize_optional_text, utc_now

from .server_control import connect
from .server_paths import ServerContext

_PASSWORD_SCRYPT_N = 1 << 14
_PASSWORD_SCRYPT_R = 8
_PASSWORD_SCRYPT_P = 1
_PASSWORD_SCRYPT_DKLEN = 64
_COMMUNITY_SESSION_TTL = timedelta(days=14)


def normalize_community_email(value: str | None) -> str | None:
    text = normalize_optional_text(value)
    if text is None:
        return None
    return text.casefold()


def community_actor_identity(account_id: str) -> str:
    return f"community:{account_id}"


def _password_params_json(*, salt: bytes) -> str:
    return json.dumps(
        {
            "salt_b64": base64.b64encode(salt).decode("ascii"),
            "n": _PASSWORD_SCRYPT_N,
            "r": _PASSWORD_SCRYPT_R,
            "p": _PASSWORD_SCRYPT_P,
            "dklen": _PASSWORD_SCRYPT_DKLEN,
        },
        sort_keys=True,
        separators=(",", ":"),
    )


def _load_password_params(raw: str) -> dict[str, Any]:
    payload = json.loads(raw)
    if not isinstance(payload, dict):
        raise ValueError("Password parameter payload must be a JSON object.")
    return payload


def _hash_password_bytes(password: str, *, params: dict[str, Any]) -> bytes:
    salt = base64.b64decode(str(params.get("salt_b64") or "").encode("ascii"))
    return hashlib.scrypt(
        password.encode("utf-8"),
        salt=salt,
        n=int(params.get("n") or _PASSWORD_SCRYPT_N),
        r=int(params.get("r") or _PASSWORD_SCRYPT_R),
        p=int(params.get("p") or _PASSWORD_SCRYPT_P),
        dklen=int(params.get("dklen") or _PASSWORD_SCRYPT_DKLEN),
    )


def hash_community_password(password: str) -> tuple[str, str, str]:
    salt = secrets.token_bytes(16)
    params_json = _password_params_json(salt=salt)
    digest = _hash_password_bytes(password, params=_load_password_params(params_json))
    return digest.hex(), "scrypt", params_json


def verify_community_password(password: str, *, password_hash: str, password_algo: str, password_params_json: str) -> bool:
    if str(password_algo or "").strip().lower() != "scrypt":
        raise ValueError(f"Unsupported Community password algorithm: {password_algo!r}")
    params = _load_password_params(password_params_json)
    expected = bytes.fromhex(str(password_hash or ""))
    actual = _hash_password_bytes(password, params=params)
    return hmac.compare_digest(expected, actual)


def _require_text(value: str | None, message: str) -> str:
    text = normalize_optional_text(value)
    if text is None:
        raise ValueError(message)
    return text


def _validate_password(password: str | None) -> str:
    text = _require_text(password, "Password is required.")
    if len(text) < 10:
        raise ValueError("Password must be at least 10 characters.")
    return text


def _expires_at_text() -> str:
    return (datetime.now(timezone.utc) + _COMMUNITY_SESSION_TTL).isoformat()


def _session_payload(account_row: dict[str, Any], session_row: dict[str, Any]) -> dict[str, Any]:
    display_name = normalize_optional_text(account_row.get("display_name")) or normalize_optional_text(account_row.get("full_name")) or account_row["email_normalized"]
    account_id = str(account_row["account_id"])
    return {
        "account_id": account_id,
        "actor_identity": community_actor_identity(account_id),
        "actor_type": "community_user",
        "display_name": display_name,
        "full_name": normalize_optional_text(account_row.get("full_name")) or display_name,
        "email_normalized": normalize_optional_text(account_row.get("email_normalized")) or "",
        "organization": normalize_optional_text(account_row.get("organization")),
        "role_title": normalize_optional_text(account_row.get("role_title")),
        "status": normalize_optional_text(account_row.get("status")) or "active",
        "primary_auth_method": normalize_optional_text(account_row.get("primary_auth_method")) or "password",
        "web_session_id": str(session_row["web_session_id"]),
        "session_source": normalize_optional_text(session_row.get("session_source")) or "password",
        "created_at": normalize_optional_text(session_row.get("created_at")) or "",
        "expires_at": normalize_optional_text(session_row.get("expires_at")) or "",
        "revoked_at": normalize_optional_text(session_row.get("revoked_at")),
        "last_seen_at": normalize_optional_text(session_row.get("last_seen_at")),
    }


def _create_session_record(conn, *, account_id: str, session_source: str) -> dict[str, Any]:
    session_id = generate_workflow_id("CWS")
    now = utc_now()
    expires_at = _expires_at_text()
    conn.execute(
        """
        insert into community_web_sessions(
            web_session_id,
            account_id,
            session_source,
            created_at,
            expires_at,
            revoked_at,
            last_seen_at
        ) values (?, ?, ?, ?, ?, ?, ?)
        """,
        (session_id, account_id, session_source, now, expires_at, None, now),
    )
    return {
        "web_session_id": session_id,
        "account_id": account_id,
        "session_source": session_source,
        "created_at": now,
        "expires_at": expires_at,
        "revoked_at": None,
        "last_seen_at": now,
    }


def create_community_account(
    ctx: ServerContext,
    *,
    full_name: str | None,
    email: str | None,
    password: str | None,
    organization: str | None = None,
    role_title: str | None = None,
) -> dict[str, Any]:
    normalized_email = normalize_community_email(email)
    if normalized_email is None:
        raise ValueError("A valid email address is required.")
    account_full_name = _require_text(full_name, "Full name is required.")
    password_text = _validate_password(password)
    account_id = generate_workflow_id("CA")
    password_hash, password_algo, password_params_json = hash_community_password(password_text)
    now = utc_now()
    with connect(ctx) as conn:
        existing = conn.execute(
            "select account_id from community_accounts where email_normalized = ? limit 1",
            (normalized_email,),
        ).fetchone()
        if existing is not None:
            raise ValueError("An account already exists for that email.")
        conn.execute(
            """
            insert into community_accounts(
                account_id,
                email_normalized,
                full_name,
                display_name,
                organization,
                role_title,
                status,
                primary_auth_method,
                created_at,
                updated_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                account_id,
                normalized_email,
                account_full_name,
                account_full_name,
                normalize_optional_text(organization),
                normalize_optional_text(role_title),
                "active",
                "password",
                now,
                now,
            ),
        )
        conn.execute(
            """
            insert into community_password_credentials(
                account_id,
                password_hash,
                password_algo,
                password_params_json,
                password_updated_at,
                must_rotate
            ) values (?, ?, ?, ?, ?, ?)
            """,
            (account_id, password_hash, password_algo, password_params_json, now, 0),
        )
        session = _create_session_record(conn, account_id=account_id, session_source="password")
        conn.commit()
    resolved = resolve_community_web_session(ctx, session["web_session_id"])
    if resolved is None:
        raise RuntimeError("Community session could not be resolved after account creation.")
    return resolved


def authenticate_community_account(ctx: ServerContext, *, email: str | None, password: str | None) -> dict[str, Any]:
    normalized_email = normalize_community_email(email)
    password_text = _validate_password(password)
    if normalized_email is None:
        raise ValueError("A valid email address is required.")
    with connect(ctx) as conn:
        row = conn.execute(
            """
            select
                a.account_id,
                a.email_normalized,
                a.full_name,
                a.display_name,
                a.organization,
                a.role_title,
                a.status,
                a.primary_auth_method,
                c.password_hash,
                c.password_algo,
                c.password_params_json
            from community_accounts a
            join community_password_credentials c on c.account_id = a.account_id
            where a.email_normalized = ?
            limit 1
            """,
            (normalized_email,),
        ).fetchone()
        if row is None:
            raise ValueError("Incorrect email or password.")
        row_dict = dict(row)
        if normalize_optional_text(row_dict.get("status")) != "active":
            raise ValueError("This Community account is not active.")
        if not verify_community_password(
            password_text,
            password_hash=str(row_dict.get("password_hash") or ""),
            password_algo=str(row_dict.get("password_algo") or ""),
            password_params_json=str(row_dict.get("password_params_json") or ""),
        ):
            raise ValueError("Incorrect email or password.")
        session = _create_session_record(conn, account_id=str(row_dict["account_id"]), session_source="password")
        conn.commit()
    resolved = resolve_community_web_session(ctx, session["web_session_id"])
    if resolved is None:
        raise RuntimeError("Community session could not be resolved after sign-in.")
    return resolved


def resolve_community_web_session(ctx: ServerContext, web_session_id: str | None) -> dict[str, Any] | None:
    session_id = normalize_optional_text(web_session_id)
    if session_id is None:
        return None
    now = datetime.now(timezone.utc).isoformat()
    with connect(ctx) as conn:
        row = conn.execute(
            """
            select
                s.web_session_id,
                s.account_id,
                s.session_source,
                s.created_at,
                s.expires_at,
                s.revoked_at,
                s.last_seen_at,
                a.email_normalized,
                a.full_name,
                a.display_name,
                a.organization,
                a.role_title,
                a.status,
                a.primary_auth_method
            from community_web_sessions s
            join community_accounts a on a.account_id = s.account_id
            where s.web_session_id = ?
              and s.revoked_at is null
              and a.status = 'active'
              and s.expires_at > ?
            limit 1
            """,
            (session_id, now),
        ).fetchone()
        if row is None:
            return None
        conn.execute(
            "update community_web_sessions set last_seen_at = ? where web_session_id = ?",
            (utc_now(), session_id),
        )
        conn.commit()
        row_dict = dict(row)
        session_row = {
            "web_session_id": row_dict["web_session_id"],
            "account_id": row_dict["account_id"],
            "session_source": row_dict["session_source"],
            "created_at": row_dict["created_at"],
            "expires_at": row_dict["expires_at"],
            "revoked_at": row_dict["revoked_at"],
            "last_seen_at": utc_now(),
        }
        return _session_payload(row_dict, session_row)


def revoke_community_web_session(ctx: ServerContext, web_session_id: str | None) -> None:
    session_id = normalize_optional_text(web_session_id)
    if session_id is None:
        return
    with connect(ctx) as conn:
        conn.execute(
            "update community_web_sessions set revoked_at = ? where web_session_id = ? and revoked_at is null",
            (utc_now(), session_id),
        )
        conn.commit()


def list_actor_role_bindings(ctx: ServerContext, actor_identity: str) -> list[dict[str, Any]]:
    identity = _require_text(actor_identity, "Actor identity is required.")
    with connect(ctx) as conn:
        rows = conn.execute(
            """
            select repo_name, role, created_at
            from role_bindings
            where actor_identity = ?
            order by repo_name, role
            """,
            (identity,),
        ).fetchall()
    return [dict(row) for row in rows]


__all__ = [
    "authenticate_community_account",
    "community_actor_identity",
    "create_community_account",
    "hash_community_password",
    "list_actor_role_bindings",
    "normalize_community_email",
    "resolve_community_web_session",
    "revoke_community_web_session",
    "verify_community_password",
]
