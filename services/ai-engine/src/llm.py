"""
OpenRouter client for the thesis engine (owner decision D-J, 17 Nov 2026).

Safety and cost rules, enforced here:
  * Key from the environment only (OPENROUTER_API_KEY); never logged or
    echoed in errors.
  * Privacy routing on every call: provider.data_collection = "deny" (no
    provider that stores/trains on data), provider.zdr = true (zero data
    retention endpoints only), provider.require_parameters = true (only
    endpoints that enforce the JSON schema). OpenRouter itself stores no
    prompts unless the account opts in (docs, checked 17 Nov 2026).
  * Structured output: response_format json_schema with strict = true.
  * SpendGuard: before a call, its worst case (estimated prompt tokens +
    max_tokens, at the live model prices) must fit the run budget, and the
    account balance must stay above a floor; after a call, the ACTUAL cost
    from `usage.cost` is booked. Over budget -> BudgetExceeded, nothing sent.
  * Pinned models with fallbacks. No `temperature`: the current models
    (Claude Haiku 5.5, gpt-5-mini) do not accept it, and with
    require_parameters it would exclude every endpoint (404, found on the
    first live call). Determinism is not relied on: every output is checked.
"""
from __future__ import annotations

import json
import os
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from typing import Dict, List, Optional

API = "https://openrouter.ai/api/v1"

# D-J model choice (17 Nov 2026, OpenRouter /models): structured outputs
# supported; Claude Haiku 5.5 ~$0.0004 per parse; fallbacks ~$0.0014-0.0017.
DEFAULT_MODELS = ["anthropic/claude-haiku-5.5", "openai/gpt-5-mini", "google/gemini-2.5-flash"]


class LLMError(RuntimeError):
    pass


class BudgetExceeded(LLMError):
    pass


def _request(method: str, path: str, key: str, body: Optional[dict] = None, timeout: float = 60.0) -> dict:
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(API + path, data=data, method=method, headers={
        "Authorization": f"Bearer {key}",
        "Content-Type": "application/json",
        "X-Title": "Constella thesis engine",
    })
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        try:
            detail = json.loads(e.read()).get("error", {}).get("message", "")
        except Exception:
            detail = ""
        raise LLMError(f"OpenRouter HTTP {e.code}: {detail[:200]}") from None
    except (urllib.error.URLError, TimeoutError) as e:
        raise LLMError(f"OpenRouter unreachable: {type(e).__name__}") from None


@dataclass
class SpendGuard:
    """Run budget in USD. Refuses a call whose worst case does not fit."""
    budget_usd: float
    min_balance_usd: float = 0.50
    spent_usd: float = 0.0
    calls: List[Dict] = field(default_factory=list)

    def worst_case(self, prompt_chars: int, max_tokens: int, price_in: float, price_out: float) -> float:
        # ~3 characters per token is a conservative (high) estimate.
        return (prompt_chars / 3.0) * price_in + max_tokens * price_out

    def admit(self, worst_case_usd: float, balance_usd: Optional[float]) -> None:
        if self.spent_usd + worst_case_usd > self.budget_usd:
            raise BudgetExceeded(f"run budget ${self.budget_usd:.4f} would be exceeded "
                                 f"(spent ${self.spent_usd:.4f}, worst case ${worst_case_usd:.4f})")
        if balance_usd is not None and balance_usd - worst_case_usd < self.min_balance_usd:
            raise BudgetExceeded(f"account balance ${balance_usd:.2f} too close to the ${self.min_balance_usd:.2f} floor")

    def book(self, model: str, cost_usd: float, usage: dict) -> None:
        self.spent_usd += cost_usd
        self.calls.append({"model": model, "cost": cost_usd,
                           "prompt_tokens": usage.get("prompt_tokens"), "completion_tokens": usage.get("completion_tokens")})


class OpenRouterClient:
    def __init__(self, guard: SpendGuard, models: Optional[List[str]] = None, key: Optional[str] = None,
                 max_tokens: int = 1200, timeout: float = 60.0):
        self.key = key if key is not None else os.environ.get("OPENROUTER_API_KEY", "")
        if not self.key:
            raise LLMError("OPENROUTER_API_KEY is not set")
        self.guard = guard
        self.models = list(models or DEFAULT_MODELS)
        self.max_tokens = max_tokens
        self.timeout = timeout
        self._prices: Optional[Dict[str, tuple]] = None

    def prices(self) -> Dict[str, tuple]:
        """USD per token (in, out) for the pinned models, from /models (free)."""
        if self._prices is None:
            data = _request("GET", "/models", self.key, timeout=self.timeout)["data"]
            table = {m["id"]: (float(m["pricing"]["prompt"]), float(m["pricing"]["completion"])) for m in data}
            missing = [m for m in self.models if m not in table]
            if missing:
                raise LLMError(f"pinned models not offered: {missing}")
            self._prices = {m: table[m] for m in self.models}
        return self._prices

    def balance(self) -> float:
        d = _request("GET", "/credits", self.key, timeout=self.timeout)["data"]
        return float(d["total_credits"]) - float(d["total_usage"])

    def chat_json(self, system: str, user: str, schema_name: str, schema: dict) -> dict:
        prices = self.prices()
        # Worst case = the most expensive pinned model (any could answer).
        p_in = max(p[0] for p in prices.values())
        p_out = max(p[1] for p in prices.values())
        self.guard.admit(self.guard.worst_case(len(system) + len(user), self.max_tokens, p_in, p_out), self.balance())
        body = {
            "model": self.models[0],
            "models": self.models,
            "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
            "max_tokens": self.max_tokens,
            "response_format": {"type": "json_schema", "json_schema": {"name": schema_name, "strict": True, "schema": schema}},
            "provider": {"require_parameters": True, "data_collection": "deny", "zdr": True},
        }
        resp = _request("POST", "/chat/completions", self.key, body, timeout=self.timeout)
        usage = resp.get("usage") or {}
        cost = usage.get("cost")
        if cost is None:  # never under-count: book the worst case if cost is missing
            cost = self.guard.worst_case(len(system) + len(user), self.max_tokens, p_in, p_out)
        self.guard.book(resp.get("model", "?"), float(cost), usage)
        try:
            choice = resp["choices"][0]
            if choice.get("finish_reason") == "length":
                raise LLMError("model output truncated at max_tokens")
            content = choice["message"]["content"]
            return json.loads(content)
        except (KeyError, IndexError, TypeError, json.JSONDecodeError) as e:
            raise LLMError(f"model returned no valid JSON ({type(e).__name__})") from None

    @property
    def last_model(self) -> str:
        return self.guard.calls[-1]["model"] if self.guard.calls else ""
