#!/usr/bin/env python3
"""Independent stdlib-only reference checker for the link-id-v1 contract.

Verifies:
1. RFC 4231 (HMAC-SHA256) and RFC 5869 (HKDF-SHA256) official test vectors.
2. Closed canonical harness vocabulary invariants in vocabulary.json.
3. All positive, collision, rotation, negative, and G0a capture-derived vectors
   in vectors/*.json (including all byte-exact intermediate values, equality
   and distinctness assertions, and G0a capture file provenance).
4. SHA-256 digests in vectors/SHA256SUMS and CONTRACT-DIGEST.

Usage:
    python -I docs/contracts/fact-export-v1/link-id-v1/check_vectors.py
"""

from __future__ import annotations

import hashlib
import hmac
import json
import pathlib
import struct
import sys
from typing import Any

CONTRACT_ID = "link-id-v1"
DOMAIN_KEY_INFO = "agent.session.link.key/v1"
DOMAIN_LINK_MESSAGE = "agent.session.link/v1"
HKDF_SALT = b"agent-otel-bridge:link-id-v1:salt"
HKDF_OUTPUT_LEN = 32
MIN_ORG_MASTER_KEY_BYTES = 32
MAX_FIELD_BYTES = 1024
UNLINKED_LITERAL = "unlinked"


def hmac_sha256(key: bytes, message: bytes) -> bytes:
    """RFC 2104 HMAC-SHA256 via Python stdlib hmac + hashlib."""
    return hmac.new(key, message, hashlib.sha256).digest()


def hkdf_extract_sha256(salt: bytes, ikm: bytes) -> bytes:
    """RFC 5869 Section 2.2 HKDF-Extract(salt, IKM) -> PRK (32 bytes)."""
    effective_salt = salt if salt else b"\x00" * 32
    return hmac_sha256(effective_salt, ikm)


def hkdf_expand_sha256(prk: bytes, info: bytes, length: int) -> bytes:
    """RFC 5869 Section 2.3 HKDF-Expand(PRK, info, L) -> OKM (length bytes)."""
    if length < 0 or length > 255 * 32:
        raise ValueError(f"invalid HKDF output length: {length}")
    n = (length + 31) // 32
    okm = bytearray()
    t_prev = b""
    for i in range(1, n + 1):
        t_prev = hmac_sha256(prk, t_prev + info + bytes([i]))
        okm.extend(t_prev)
    return bytes(okm[:length])


def hkdf_sha256(ikm: bytes, salt: bytes, info: bytes, length: int = HKDF_OUTPUT_LEN) -> bytes:
    """RFC 5869 HKDF-SHA256."""
    prk = hkdf_extract_sha256(salt, ikm)
    return hkdf_expand_sha256(prk, info, length)


def lp_encode_bytes(fields: list[bytes]) -> bytes:
    """Encode an ordered tuple of byte strings with u32 big-endian length prefixes."""
    out = bytearray()
    for field in fields:
        out.extend(struct.pack(">I", len(field)))
        out.extend(field)
    return bytes(out)


def lp_encode_strs(fields: list[str]) -> bytes:
    """Encode an ordered tuple of UTF-8 strings with u32 big-endian length prefixes."""
    return lp_encode_bytes([f.encode("utf-8", errors="strict") for f in fields])


UNICODE_WHITESPACE: frozenset[str] = frozenset(
    [
        "\u0009",  # CHARACTER TABULATION
        "\u000a",  # LINE FEED
        "\u000b",  # LINE TABULATION
        "\u000c",  # FORM FEED
        "\u000d",  # CARRIAGE RETURN
        "\u0020",  # SPACE
        "\u0085",  # NEXT LINE (NEL)
        "\u00a0",  # NO-BREAK SPACE
        "\u1680",  # OGHAM SPACE MARK
        "\u2000",  # EN QUAD
        "\u2001",  # EM QUAD
        "\u2002",  # EN SPACE
        "\u2003",  # EM SPACE
        "\u2004",  # THREE-PER-EM SPACE
        "\u2005",  # FOUR-PER-EM SPACE
        "\u2006",  # SIX-PER-EM SPACE
        "\u2007",  # FIGURE SPACE
        "\u2008",  # PUNCTUATION SPACE
        "\u2009",  # THIN SPACE
        "\u200a",  # HAIR SPACE
        "\u2028",  # LINE SEPARATOR
        "\u2029",  # PARAGRAPH SEPARATOR
        "\u202f",  # NARROW NO-BREAK SPACE
        "\u205f",  # MEDIUM MATHEMATICAL SPACE
        "\u3000",  # IDEOGRAPHIC SPACE
    ]
)


def is_unicode_whitespace_only(val: str) -> bool:
    """Return True if val is empty or every code point is in the 25-char Unicode White_Space set."""
    return len(val) == 0 or all(ch in UNICODE_WHITESPACE for ch in val)


def is_valid_ascii_slug(val: str) -> bool:
    """Check SPEC.md §4.3 ASCII slug grammar [\\x21-\\x7E]{1,1024} (printable ASCII, no spaces)."""
    raw = val.encode("utf-8", errors="strict")
    if len(raw) == 0 or len(raw) > MAX_FIELD_BYTES:
        return False
    return all(0x21 <= b <= 0x7E for b in raw)


def is_valid_native_session(val: str) -> bool:
    """Check SPEC.md §4.3 native_session rules (1..=1024 UTF-8 bytes, no C0/C1 ctl, no outer Unicode White_Space)."""
    raw = val.encode("utf-8", errors="strict")
    if len(raw) == 0 or len(raw) > MAX_FIELD_BYTES:
        return False
    if val[0] in UNICODE_WHITESPACE or val[-1] in UNICODE_WHITESPACE:
        return False
    for ch in val:
        cp = ord(ch)
        if cp <= 0x001F or (0x007F <= cp <= 0x009F):
            return False
    return True


def normalize_harness_alias(raw: str) -> str | None:
    """ASCII lowercase of an ASCII-only input without trimming; non-ASCII returns None."""
    try:
        raw_bytes = raw.encode("ascii")
    except UnicodeEncodeError:
        return None
    return raw_bytes.lower().decode("ascii")


def verify_rfc_vectors() -> None:
    """Verify RFC 4231 (HMAC-SHA256) and RFC 5869 (HKDF-SHA256) official test vectors."""
    rfc4231_cases = [
        (
            bytes([0x0B] * 20),
            b"Hi There",
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
        ),
        (
            b"Jefe",
            b"what do ya want for nothing?",
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
        ),
        (
            bytes([0xAA] * 20),
            bytes([0xDD] * 50),
            "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
        ),
        (
            bytes.fromhex("0102030405060708090a0b0c0d0e0f10111213141516171819"),
            bytes([0xCD] * 50),
            "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b",
        ),
        (
            bytes([0xAA] * 131),
            b"Test Using Larger Than Block-Size Key - Hash Key First",
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
        ),
        (
            bytes([0xAA] * 131),
            b"This is a test using a larger than block-size key and a larger than block-size data. The key needs to be hashed before being used by the HMAC algorithm.",
            "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2",
        ),
    ]
    for idx, (key, data, expected_hex) in enumerate(rfc4231_cases, start=1):
        actual_hex = hmac_sha256(key, data).hex()
        if actual_hex != expected_hex:
            raise AssertionError(f"RFC 4231 case {idx} failed: {actual_hex} != {expected_hex}")

    rfc5869_cases = [
        (
            bytes([0x0B] * 22),
            bytes.fromhex("000102030405060708090a0b0c"),
            bytes.fromhex("f0f1f2f3f4f5f6f7f8f9"),
            42,
            "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5",
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865",
        ),
        (
            bytes(range(0x00, 0x50)),
            bytes(range(0x60, 0xB0)),
            bytes(range(0xB0, 0x100)),
            82,
            "06a6b88c5853361a06104c9ceb35b45cef760014904671014a193f40c15fc244",
            (
                "b11e398dc80327a1c8e7f78c596a49344f012eda2d4efad8a050cc4c19afa97c"
                "59045a99cac7827271cb41c65e590e09da3275600c2f09b8367793a9aca3db71"
                "cc30c58179ec3e87c14c01d5c1f3434f1d87"
            ),
        ),
        (
            bytes([0x0B] * 22),
            b"",
            b"",
            42,
            "19ef24a32c717b167f33a91d6f648bdf96596776afdb6377ac434c1c293ccb04",
            "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d9d201395faa4b61a96c8",
        ),
    ]
    for idx, (ikm, salt, info, length, expected_prk, expected_okm) in enumerate(rfc5869_cases, start=1):
        prk = hkdf_extract_sha256(salt, ikm)
        okm = hkdf_expand_sha256(prk, info, length)
        if prk.hex() != expected_prk:
            raise AssertionError(f"RFC 5869 A.{idx} PRK mismatch: {prk.hex()} != {expected_prk}")
        if okm.hex() != expected_okm:
            raise AssertionError(f"RFC 5869 A.{idx} OKM mismatch: {okm.hex()} != {expected_okm}")


def load_and_verify_vocabulary(vocab_path: pathlib.Path) -> tuple[dict[str, str], dict[int, str]]:
    """Load vocabulary.json and verify closed-world alias and wire-ID invariants."""
    vocab = json.loads(vocab_path.read_text(encoding="utf-8"))
    if vocab.get("contract") != CONTRACT_ID:
        raise AssertionError(f"Unexpected vocabulary contract: {vocab.get('contract')}")
    if vocab.get("unknown_behavior") != UNLINKED_LITERAL:
        raise AssertionError(f"Unexpected unknown_behavior: {vocab.get('unknown_behavior')}")

    alias_to_canonical: dict[str, str] = {}
    wire_to_canonical: dict[int, str] = {}
    expected_canonicals = {"antigravity", "claude", "codex", "grok", "pi"}
    seen_canonicals: set[str] = set()

    for entry in vocab["canonical_harnesses"]:
        canonical = entry["canonical"]
        wire_id = entry["wire_client_id"]
        aliases = entry["aliases"]
        if canonical in seen_canonicals:
            raise AssertionError(f"Duplicate canonical harness: {canonical}")
        seen_canonicals.add(canonical)
        if wire_id in wire_to_canonical:
            raise AssertionError(f"Duplicate wire_client_id: {wire_id}")
        wire_to_canonical[wire_id] = canonical
        if canonical not in aliases:
            raise AssertionError(f"Canonical {canonical} missing from its own aliases list")
        for alias in aliases:
            norm = normalize_harness_alias(alias)
            if norm is None or norm != alias or not is_valid_ascii_slug(alias):
                raise AssertionError(f"Invalid alias token in vocabulary.json: {alias!r}")
            if norm in alias_to_canonical:
                raise AssertionError(f"Alias collision on {norm!r}")
            alias_to_canonical[norm] = canonical

    if seen_canonicals != expected_canonicals:
        raise AssertionError(f"Canonical set mismatch: {seen_canonicals} != {expected_canonicals}")

    if vocab.get("aliases") != alias_to_canonical:
        raise AssertionError("Top-level aliases map does not match canonical_harnesses aliases")

    expected_wire_map = {str(k): v for k, v in wire_to_canonical.items()}
    if vocab.get("wire_client_ids") != expected_wire_map:
        raise AssertionError("Top-level wire_client_ids map does not match canonical_harnesses")

    if alias_to_canonical.get("claude-code") != "claude" or alias_to_canonical.get("claude") != "claude":
        raise AssertionError("F12 violation: claude-code and claude must both resolve to 'claude'")

    for removed_alias in ("claudecode", "codex-cli", "grok-cli", "pi-cli"):
        if removed_alias in alias_to_canonical:
            raise AssertionError(f"Removed un-emitted alias {removed_alias!r} present in alias table")

    for provider in ("google", "anthropic", "openai", "xai", "gemini", "inflection", "ai-agent", "unknown"):
        if provider in alias_to_canonical:
            raise AssertionError(f"Forbidden provider/fallback name {provider!r} present in alias table")

    return alias_to_canonical, wire_to_canonical


def resolve_harness(
    wire_client_id: int | None,
    raw_harness: str | None,
    alias_to_canonical: dict[str, str],
    wire_to_canonical: dict[int, str],
) -> tuple[str | None, str | None]:
    """Resolve canonical harness from wire_client_id and/or raw_harness."""
    wire_canon: str | None = None
    if wire_client_id is not None and wire_client_id != 0:
        if wire_client_id not in wire_to_canonical:
            return None, "unknown_harness"
        wire_canon = wire_to_canonical[wire_client_id]

    str_canon: str | None = None
    if raw_harness is not None:
        if is_unicode_whitespace_only(raw_harness):
            if wire_canon is None:
                return None, "missing_harness"
        else:
            lower = normalize_harness_alias(raw_harness)
            if lower is None or lower not in alias_to_canonical:
                return None, "unknown_harness"
            str_canon = alias_to_canonical[lower]

    if wire_canon is not None and str_canon is not None:
        if wire_canon != str_canon:
            return None, "conflicting_harness"
        return wire_canon, None
    if wire_canon is not None:
        return wire_canon, None
    if str_canon is not None:
        return str_canon, None
    return None, "missing_harness"


def evaluate_vector(
    vec: dict[str, Any],
    alias_to_canonical: dict[str, str],
    wire_to_canonical: dict[int, str],
) -> tuple[dict[str, Any], dict[str, Any]]:
    """Evaluate a vector entry and return (computed_intermediates, computed_expected)."""
    inp = vec["inputs"]
    if inp.get("salt_hex") != HKDF_SALT.hex():
        raise AssertionError(f"Vector {vec['id']} has non-normative salt_hex: {inp.get('salt_hex')}")

    k_org = bytes.fromhex(inp["k_org_hex"])
    team = inp.get("team")
    epoch = inp.get("epoch")
    org = inp.get("org")
    endpoint_scope = inp.get("endpoint_scope")
    wire_client_id = inp.get("wire_client_id")
    raw_harness = inp.get("raw_harness")
    native_session = inp.get("native_session")
    origin = inp.get("origin")

    hkdf_info_hex: str | None = None
    prk_hex: str | None = None
    k_link_hex: str | None = None
    canonical_harness: str | None = None
    lp_payload_hex: str | None = None
    expected_link_id = UNLINKED_LITERAL
    unlinked_reason: str | None = None

    if (
        len(k_org) < MIN_ORG_MASTER_KEY_BYTES
        or team is None
        or epoch is None
        or not is_valid_ascii_slug(team)
        or not is_valid_ascii_slug(epoch)
    ):
        unlinked_reason = "invalid_key_material"
    else:
        hkdf_info = lp_encode_strs([DOMAIN_KEY_INFO, team, epoch])
        prk = hkdf_extract_sha256(HKDF_SALT, k_org)
        k_link = hkdf_expand_sha256(prk, hkdf_info, HKDF_OUTPUT_LEN)
        hkdf_info_hex = hkdf_info.hex()
        prk_hex = prk.hex()
        k_link_hex = k_link.hex()

        if origin is not None and origin != "harness":
            unlinked_reason = "non_harness_origin"
        elif org is None or is_unicode_whitespace_only(org):
            unlinked_reason = "missing_org"
        elif not is_valid_ascii_slug(org):
            unlinked_reason = "invalid_field_characters"
        elif endpoint_scope is None or is_unicode_whitespace_only(endpoint_scope):
            unlinked_reason = "missing_endpoint_scope"
        elif not is_valid_ascii_slug(endpoint_scope):
            unlinked_reason = "invalid_field_characters"
        else:
            canon, h_err = resolve_harness(
                wire_client_id, raw_harness, alias_to_canonical, wire_to_canonical
            )
            if h_err is not None:
                unlinked_reason = h_err
            else:
                canonical_harness = canon
                if native_session is None or is_unicode_whitespace_only(native_session):
                    unlinked_reason = "missing_native_session"
                elif not is_valid_native_session(native_session):
                    unlinked_reason = "invalid_field_characters"
                else:
                    assert canonical_harness is not None
                    lp_payload = lp_encode_strs(
                        [
                            DOMAIN_LINK_MESSAGE,
                            org,
                            endpoint_scope,
                            canonical_harness,
                            native_session,
                        ]
                    )
                    lp_payload_hex = lp_payload.hex()
                    expected_link_id = hmac_sha256(k_link, lp_payload).hex()

    computed_intermediates = {
        "hkdf_info_hex": hkdf_info_hex,
        "prk_hex": prk_hex,
        "k_link_hex": k_link_hex,
        "canonical_harness": canonical_harness,
        "lp_payload_hex": lp_payload_hex,
    }
    computed_expected = {
        "link_id": expected_link_id,
        "unlinked_reason": unlinked_reason,
    }
    return computed_intermediates, computed_expected


def verify_g0a_capture_provenance(base_dir: pathlib.Path, g0a_doc: dict[str, Any]) -> None:
    """Verify that every G0a vector references existing capture files with matching SHA-256 and content."""
    repo_root = base_dir.parent.parent.parent.parent
    captures_sha_path = (
        repo_root / "docs" / "contracts" / "fact-export-v1" / "captures" / "g0a-20261112" / "SHA256SUMS"
    )
    manifest_hashes: dict[str, str] = {}
    for line in captures_sha_path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        digest, rel = line.split("  ", 1)
        manifest_hashes[rel] = digest

    file_keys = [
        ("hook_file", "hook_file_sha256"),
        ("traces_file", "traces_file_sha256"),
        ("session_log_file", "session_log_file_sha256"),
    ]
    for vec in g0a_doc["vectors"]:
        prov = vec["g0a_provenance"]
        native_session = vec["inputs"]["native_session"]
        verified_any_file = False
        for path_key, sha_key in file_keys:
            if path_key not in prov:
                continue
            verified_any_file = True
            capture_rel = prov[path_key]
            expected_sha = prov[sha_key]
            capture_path = repo_root / capture_rel
            if not capture_path.is_file():
                raise AssertionError(f"Missing G0a capture file: {capture_path}")
            actual_sha = hashlib.sha256(capture_path.read_bytes()).hexdigest()
            if actual_sha != expected_sha:
                raise AssertionError(
                    f"G0a capture SHA-256 mismatch for {capture_rel}: {actual_sha} != {expected_sha}"
                )
            short_rel = capture_rel.split("docs/contracts/fact-export-v1/captures/g0a-20261112/", 1)[1]
            if manifest_hashes.get(short_rel) != actual_sha:
                raise AssertionError(f"G0a capture {short_rel} does not match captures SHA256SUMS")
            text = capture_path.read_text(encoding="utf-8")
            if native_session not in text:
                raise AssertionError(
                    f"Vector {vec['id']} native_session {native_session!r} not found in {capture_rel}"
                )
        if not verified_any_file:
            raise AssertionError(f"Vector {vec['id']} has no capture files in g0a_provenance")


def verify_vector_files(
    base_dir: pathlib.Path,
    alias_to_canonical: dict[str, str],
    wire_to_canonical: dict[int, str],
) -> int:
    """Verify all vector JSON files in vectors/ and return total verified vector count."""
    vectors_dir = base_dir / "vectors"
    expected_files = ["basic.json", "collisions.json", "rotation.json", "negative.json", "g0a.json"]
    total_vectors = 0

    for filename in expected_files:
        path = vectors_dir / filename
        doc = json.loads(path.read_text(encoding="utf-8"))
        if doc.get("contract") != CONTRACT_ID:
            raise AssertionError(f"{filename}: unexpected contract {doc.get('contract')}")

        by_id: dict[str, dict[str, Any]] = {}
        for vec in doc["vectors"]:
            vid = vec["id"]
            if vid in by_id:
                raise AssertionError(f"{filename}: duplicate vector id {vid}")
            by_id[vid] = vec

            comp_inter, comp_exp = evaluate_vector(vec, alias_to_canonical, wire_to_canonical)
            if vec["intermediates"] != comp_inter:
                raise AssertionError(
                    f"{filename}:{vid}: intermediates mismatch:\n  got:      {comp_inter}\n  expected: {vec['intermediates']}"
                )
            if vec["expected"] != comp_exp:
                raise AssertionError(
                    f"{filename}:{vid}: expected mismatch:\n  got:      {comp_exp}\n  expected: {vec['expected']}"
                )
            total_vectors += 1

        for group in doc.get("assert_equal_link_id_groups", []):
            ids = [by_id[vid]["expected"]["link_id"] for vid in group]
            if len(set(ids)) != 1 or ids[0] == UNLINKED_LITERAL:
                raise AssertionError(f"{filename}: assert_equal_link_id_groups failed for {group}: {ids}")

        for group in doc.get("assert_distinct_link_id_groups", []):
            ids = [by_id[vid]["expected"]["link_id"] for vid in group]
            if len(set(ids)) != len(ids):
                raise AssertionError(f"{filename}: assert_distinct_link_id_groups failed for {group}: {ids}")

        for group in doc.get("assert_distinct_k_link_groups", []):
            keys = [by_id[vid]["intermediates"]["k_link_hex"] for vid in group]
            if len(set(keys)) != len(keys):
                raise AssertionError(f"{filename}: assert_distinct_k_link_groups failed for {group}")

        if "overlap_window_example" in doc:
            ov = doc["overlap_window_example"]
            ref_vec = by_id["rotation_epoch_2026_q4"]["inputs"]
            k_org = bytes.fromhex(ref_vec["k_org_hex"])
            candidates = []
            for ep in ov["active_epochs"]:
                info_bytes = lp_encode_strs([DOMAIN_KEY_INFO, ref_vec["team"], ep])
                k_link = hkdf_sha256(k_org, HKDF_SALT, info_bytes, HKDF_OUTPUT_LEN)
                msg = lp_encode_strs(
                    [
                        DOMAIN_LINK_MESSAGE,
                        ref_vec["org"],
                        ref_vec["endpoint_scope"],
                        alias_to_canonical[ref_vec["raw_harness"]],
                        ref_vec["native_session"],
                    ]
                )
                candidates.append({"epoch": ep, "link_id": hmac_sha256(k_link, msg).hex()})
            if candidates != ov["expected_candidates"]:
                raise AssertionError(
                    f"{filename}: overlap_window_example mismatch: {candidates} != {ov['expected_candidates']}"
                )

        if filename == "g0a.json":
            verify_g0a_capture_provenance(base_dir, doc)

    return total_vectors


def verify_digests(base_dir: pathlib.Path) -> str:
    """Verify vectors/SHA256SUMS and CONTRACT-DIGEST."""
    vectors_sha_path = base_dir / "vectors" / "SHA256SUMS"
    for line in vectors_sha_path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        expected_sha, filename = line.split("  ", 1)
        actual_sha = hashlib.sha256((base_dir / "vectors" / filename).read_bytes()).hexdigest()
        if actual_sha != expected_sha:
            raise AssertionError(
                f"vectors/SHA256SUMS mismatch for {filename}: {actual_sha} != {expected_sha}"
            )

    digest_path = base_dir / "CONTRACT-DIGEST"
    file_entries: list[str] = []
    declared_bundle_sha: str | None = None
    for line in digest_path.read_text(encoding="utf-8").splitlines():
        s = line.strip()
        if not s:
            continue
        if s.startswith("# BUNDLE-SHA256:"):
            declared_bundle_sha = s.split(":", 1)[1].strip()
            continue
        if s.startswith("#"):
            continue
        expected_sha, rel_path = s.split("  ", 1)
        actual_sha = hashlib.sha256((base_dir / rel_path).read_bytes()).hexdigest()
        if actual_sha != expected_sha:
            raise AssertionError(
                f"CONTRACT-DIGEST mismatch for {rel_path}: {actual_sha} != {expected_sha}"
            )
        file_entries.append(f"{expected_sha}  {rel_path}\n")

    if not file_entries or declared_bundle_sha is None:
        raise AssertionError("CONTRACT-DIGEST missing file entries or BUNDLE-SHA256 footer")

    computed_bundle_sha = hashlib.sha256("".join(file_entries).encode("utf-8")).hexdigest()
    if computed_bundle_sha != declared_bundle_sha:
        raise AssertionError(
            f"CONTRACT-DIGEST BUNDLE-SHA256 mismatch: {computed_bundle_sha} != {declared_bundle_sha}"
        )
    return computed_bundle_sha


def main() -> int:
    base_dir = pathlib.Path(__file__).resolve().parent
    verify_rfc_vectors()
    alias_to_canonical, wire_to_canonical = load_and_verify_vocabulary(base_dir / "vocabulary.json")
    total_vectors = verify_vector_files(base_dir, alias_to_canonical, wire_to_canonical)
    bundle_sha = verify_digests(base_dir)
    print(
        f"OK: link-id-v1 verified (RFC 4231/5869, vocabulary, {total_vectors} vectors, "
        f"G0a provenance, SHA256SUMS, CONTRACT-DIGEST bundle={bundle_sha})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
