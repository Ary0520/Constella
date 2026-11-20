"""Request validation for the Constella sponsor gateway (audit F1).

The gateway pays network fees for users who hold no XLM (passkey smart
accounts). A sponsored request is a Soroban host function plus the user's
signed authorization entries (the same `func` + `auth` shape as the
OpenZeppelin Relayer Channels plugin). The relayer's own account is the
transaction source, so the dangerous request is one that makes the relayer
authorize something - e.g. a token transfer out of the relayer's account.

Rules (all must hold, otherwise `Rejected` with a stable code):
  1. `func` is valid XDR for an InvokeContract host function (no contract
     creation, no WASM upload).
  2. The target contract and function are on the allow-list.
  3. There is at least one, and at most MAX_AUTH_ENTRIES, auth entries.
  4. Every auth entry uses address credentials - `ADDRESS`, `ADDRESS_V2`
     (CAP-71: the signed payload is bound to the address) or
     `ADDRESS_WITH_DELEGATES` (CAP-71 delegate signatures, bounded to
     MAX_DELEGATE_NODES). SourceAccount credentials would let the
     transaction source (the relayer) authorize - refused.
  5. No relayer-controlled address appears as an auth address, as a
     delegate at any depth, or anywhere in the call arguments.
  6. The first entry's root invocation is exactly this call (contract,
     function, arguments); its address is the sponsored user.

Validation is pure (no network); on-chain checks (whitelist, quotas, fee cap
after simulation) happen in `relayer.py`.
"""
from __future__ import annotations

import base64
import binascii
from dataclasses import dataclass

from stellar_sdk import Address, xdr

MAX_AUTH_ENTRIES = 4
MAX_DELEGATE_NODES = 8
MAX_FUNC_XDR_BYTES = 16 * 1024
MAX_AUTH_XDR_BYTES = 16 * 1024


class Rejected(ValueError):
    """A request the gateway will not sponsor. `code` is stable for clients."""

    def __init__(self, code: str, detail: str = ""):
        super().__init__(f"{code}: {detail}" if detail else code)
        self.code = code
        self.detail = detail


@dataclass(frozen=True)
class Validated:
    contract: str
    function: str
    user: str
    host_function: xdr.HostFunction
    auth: list[xdr.SorobanAuthorizationEntry]


def _decode(b64: str, limit: int, kind: str) -> bytes:
    if not isinstance(b64, str):
        raise Rejected("INVALID_PARAMS", f"{kind} must be a base64 string")
    try:
        raw = base64.b64decode(b64, validate=True)
    except (binascii.Error, ValueError) as e:
        raise Rejected("INVALID_XDR", f"{kind}: not base64 ({e})") from None
    if len(raw) > limit:
        raise Rejected("INVALID_PARAMS", f"{kind} larger than {limit} bytes")
    return raw


def _addresses_in(val: xdr.SCVal, out: set[str]) -> None:
    """Collect every address inside an SCVal (vectors and maps included)."""
    t = val.type
    if t == xdr.SCValType.SCV_ADDRESS:
        out.add(Address.from_xdr_sc_address(val.address).address)
    elif t == xdr.SCValType.SCV_VEC and val.vec is not None:
        for v in val.vec.sc_vec:
            _addresses_in(v, out)
    elif t == xdr.SCValType.SCV_MAP and val.map is not None:
        for e in val.map.sc_map:
            _addresses_in(e.key, out)
            _addresses_in(e.val, out)


_ADDRESS_TYPES = (
    xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_ADDRESS,
    xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_ADDRESS_V2,
    xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_ADDRESS_WITH_DELEGATES,
)


def address_credentials(creds: xdr.SorobanCredentials) -> xdr.SorobanAddressCredentials | None:
    """The top-level address credentials of any address credential type."""
    t = creds.type
    if t == xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_ADDRESS:
        return creds.address
    if t == xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_ADDRESS_V2:
        return creds.address_v2
    if t == xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_ADDRESS_WITH_DELEGATES:
        return creds.address_with_delegates.address_credentials
    return None


def _signers(creds: xdr.SorobanCredentials, i: int) -> list[str]:
    """Every address that signs an entry: the account, then all delegates
    (depth-first). Bounded so validation stays cheap."""
    top = address_credentials(creds)
    out = [Address.from_xdr_sc_address(top.address).address]
    if creds.type == xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_ADDRESS_WITH_DELEGATES:
        stack = list(creds.address_with_delegates.delegates)
        while stack:
            d = stack.pop()
            out.append(Address.from_xdr_sc_address(d.address).address)
            if len(out) > 1 + MAX_DELEGATE_NODES:
                raise Rejected("TOO_MANY_DELEGATES", f"auth[{i}]")
            stack.extend(d.nested_delegates)
    return out


def _same_call(a: xdr.InvokeContractArgs, b: xdr.InvokeContractArgs) -> bool:
    return a.to_xdr_bytes() == b.to_xdr_bytes()


def validate(
    func_b64: str,
    auth_b64: list[str],
    allowed: dict[str, frozenset[str]],
    relayer_addresses: frozenset[str],
) -> Validated:
    """Validate a sponsored request. `allowed` maps contract IDs to the
    function names that may be sponsored; `relayer_addresses` are every
    account the relayer controls (source / fee-paying / channel accounts)."""
    try:
        host_fn = xdr.HostFunction.from_xdr_bytes(_decode(func_b64, MAX_FUNC_XDR_BYTES, "func"))
    except Rejected:
        raise
    except Exception as e:  # malformed XDR of any kind
        raise Rejected("INVALID_XDR", f"func: {e}") from None
    if host_fn.type != xdr.HostFunctionType.HOST_FUNCTION_TYPE_INVOKE_CONTRACT:
        raise Rejected("NOT_A_CONTRACT_CALL", str(host_fn.type))
    call = host_fn.invoke_contract
    contract = Address.from_xdr_sc_address(call.contract_address).address
    function = call.function_name.sc_symbol.decode()
    if contract not in allowed:
        raise Rejected("TARGET_NOT_ALLOWED", contract)
    if function not in allowed[contract]:
        raise Rejected("FUNCTION_NOT_ALLOWED", f"{contract}.{function}")

    if not isinstance(auth_b64, list) or not auth_b64:
        raise Rejected("AUTH_REQUIRED", "a sponsored call must be authorized by the user")
    if len(auth_b64) > MAX_AUTH_ENTRIES:
        raise Rejected("TOO_MANY_AUTH_ENTRIES", str(len(auth_b64)))
    entries: list[xdr.SorobanAuthorizationEntry] = []
    for i, b64 in enumerate(auth_b64):
        try:
            entries.append(xdr.SorobanAuthorizationEntry.from_xdr_bytes(_decode(b64, MAX_AUTH_XDR_BYTES, f"auth[{i}]")))
        except Rejected:
            raise
        except Exception as e:
            raise Rejected("INVALID_XDR", f"auth[{i}]: {e}") from None

    for i, entry in enumerate(entries):
        ctype = entry.credentials.type
        if ctype == xdr.SorobanCredentialsType.SOROBAN_CREDENTIALS_SOURCE_ACCOUNT:
            raise Rejected("SOURCE_ACCOUNT_AUTH_FORBIDDEN", f"auth[{i}] would be authorized by the relayer's account")
        if ctype not in _ADDRESS_TYPES:
            raise Rejected("UNSUPPORTED_CREDENTIALS", f"auth[{i}]: {ctype}")
        if set(_signers(entry.credentials, i)) & relayer_addresses:
            raise Rejected("RELAYER_ADDRESS_IN_AUTH", f"auth[{i}]")

    mentioned: set[str] = set()
    for arg in call.args:
        _addresses_in(arg, mentioned)
    if mentioned & relayer_addresses:
        raise Rejected("RELAYER_ADDRESS_IN_ARGS", ", ".join(sorted(mentioned & relayer_addresses)))

    root = entries[0].root_invocation.function
    if root.type != xdr.SorobanAuthorizedFunctionType.SOROBAN_AUTHORIZED_FUNCTION_TYPE_CONTRACT_FN or not _same_call(
        root.contract_fn, call
    ):
        raise Rejected("AUTH_MISMATCH", "auth[0] does not authorize this exact call")
    user = Address.from_xdr_sc_address(address_credentials(entries[0].credentials).address).address
    return Validated(contract=contract, function=function, user=user, host_function=host_fn, auth=entries)
