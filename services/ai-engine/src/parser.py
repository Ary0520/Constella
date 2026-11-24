"""
Parser module for the Constella AI Thesis Engine.
Converts plain-English investment statements into typed, validated PolicyDraft schemas.
Enforces closed universe awareness and conservative defaults for ambiguous inputs.
"""

import re
from typing import Dict, List, Set
from src.models import PolicyDraft


# Known unregistered tickers and cryptos outside the closed Stellar asset registry
UNREGISTERED_SYMBOLS = {
    "TSLA", "TESLA", "BTC", "BITCOIN", "ETH", "ETHEREUM",
    "SOL", "SOLANA", "AAPL", "APPLE", "AMZN", "AMAZON",
    "GOOG", "GOOGLE", "NVDA", "NVIDIA", "MSFT", "MICROSOFT"
}


# REGISTRY.md "Issuer Concentration Policy": defaults, user-configurable.
DEFAULT_MAX_ISSUER = 0.30
DEFAULT_MAX_SINGLE_ASSET = 0.25

# Clauses: split on , ; " and " " but " and on a period that ends a sentence
# (not the one inside "22.5%").
_CLAUSE_SPLIT = re.compile(r"[;,]|\.(?=\s|$)|\band\b|\bbut\b", re.IGNORECASE)
_PCT = re.compile(r"(\d+(?:\.\d+)?)\s*%")
# Step 4: fraction words read as percentages ("hold a third in euro" is the
# founding doc's own example). Word boundaries keep "behalf" etc. intact.
_FRACTIONS = [
    (re.compile(r"\btwo[\s-]+thirds\b", re.I), "66.67%"),
    (re.compile(r"\bthree[\s-]+quarters\b", re.I), "75%"),
    (re.compile(r"\b(?:a|one)[\s-]+third\b", re.I), "33.33%"),
    (re.compile(r"\b(?:a|one)[\s-]+quarter\b", re.I), "25%"),
    (re.compile(r"\b(?:a|one)?[\s-]*half\b", re.I), "50%"),
]


def _fractions_as_percent(text: str) -> str:
    for rx, pct in _FRACTIONS:
        text = rx.sub(pct, text)
    return text
_MAX_WORDS = re.compile(r"no more than|not more than|at most|\bmax(?:imum)?\b|\bup to\b|\bcap(?:ped)?\b|\blimit(?:ed)?\b|\babove\b|\bover\b|\bceiling\b|<=|≤")
_MIN_WORDS = re.compile(r"at least|\bminimum\b|\bmin\b|no less than|not less than")
_ISSUER = re.compile(r"\bissuers?\b")
_ASSET = re.compile(r"\b(?:single|any|each|one|per)\s+(?:single\s+)?(?:assets?|tokens?|holdings?|positions?)\b")


def _fmt(v: float) -> str:
    return f"{v * 100:g}%"


def _resolve_cap(caps: List[float], default: float, kind: str, ambiguities: List[str], confidence: Dict[str, float], key: str) -> float:
    """Apply the user's cap: the strictest if several conflict; a cap looser
    than the documented default is applied but flagged for confirmation."""
    if not caps:
        return default
    cap = min(caps)
    if len(set(caps)) > 1:
        ambiguities.append(f"Conflicting {kind} caps ({', '.join(_fmt(c) for c in caps)}); applied the strictest, {_fmt(cap)}.")
    if cap > default:
        ambiguities.append(f"Requested {kind} cap {_fmt(cap)} is looser than the {_fmt(default)} default; please confirm on the review screen.")
    confidence[key] = 0.95
    return cap


def parse_thesis(
    thesis: str,
    jurisdiction: str = "non-US",
    investor_class: str = "retail"
) -> PolicyDraft:
    """
    Parse a plain-English investment thesis into a typed PolicyDraft.
    
    Offline deterministic execution:
    1. Extracts currency exposures and liquidity reserve requirements.
    2. Identifies target objectives and risk orientation.
    3. Flags unsupported assets outside the closed registry in ambiguities.
    4. Applies conservative fallback parameters and confidence penalties for vague inputs.
    """
    raw_text = _fractions_as_percent(thesis.strip())
    text_lower = raw_text.lower()
    ambiguities: List[str] = []
    confidence: Dict[str, float] = {}

    # Check for prompt vagueness (< 5 words or very short)
    is_vague = len(raw_text.split()) < 5

    # 1. Detect unsupported assets (Closed Universe Rule)
    # Check for known unregistered tickers/names
    for symbol in UNREGISTERED_SYMBOLS:
        pattern = rf"\b{re.escape(symbol.lower())}\b"
        if re.search(pattern, text_lower):
            ambiguities.append(
                f"Requested asset '{symbol}' is not part of the verified Stellar asset registry (Closed Universe Rule)."
            )
            confidence["unregistered_asset_detected"] = 0.20

    # 2. Percentages, read clause by clause (audit A2): each percentage is
    # interpreted with its own subject (issuer / asset / cash / currency) and
    # direction ("at most" vs "at least"); the user's caps reach the policy.
    currency_exposure: Dict[str, float] = {}
    min_liquidity_sleeve: float = 0.10
    has_explicit_liquidity = False
    issuer_caps: List[float] = []
    asset_caps: List[float] = []

    for clause in _CLAUSE_SPLIT.split(raw_text):
        cl = clause.lower()
        for m in _PCT.finditer(clause):
            pct = float(m.group(1)) / 100.0
            is_max = bool(_MAX_WORDS.search(cl))
            is_min = bool(_MIN_WORDS.search(cl))
            if _ISSUER.search(cl) or _ASSET.search(cl):
                kind = "issuer" if _ISSUER.search(cl) else "single-asset"
                if is_min and not is_max:
                    ambiguities.append(f"A minimum per {kind.replace('single-', '')} ({_fmt(pct)}) is not supported; caps are maximums. Kept the default.")
                elif not (0.0 < pct <= 1.0):
                    ambiguities.append(f"Ignored impossible {kind} cap {_fmt(pct)}; kept the default.")
                else:
                    (issuer_caps if kind == "issuer" else asset_caps).append(pct)
                continue
            ctx_lower = cl[m.end():] + " " + cl[: m.start()]
            # Check for liquidity sleeve
            if any(term in ctx_lower for term in ["liquidity", "instant liquidity", "cash", "reserve", "buffer"]):
                if is_max and not is_min:
                    ambiguities.append(f"A maximum cash share ({_fmt(pct)}) is not supported; the cash sleeve is a minimum (liquidity floor). Kept the default floor.")
                    continue
                if has_explicit_liquidity and pct != min_liquidity_sleeve:
                    ambiguities.append(f"Conflicting cash reserves ({_fmt(min_liquidity_sleeve)}, {_fmt(pct)}); applied the higher.")
                # Several readings: the higher floor is the safer one.
                min_liquidity_sleeve = max(pct, min_liquidity_sleeve) if has_explicit_liquidity else pct
                has_explicit_liquidity = True
                confidence["min_liquidity_sleeve"] = 0.95
                if pct < 0.10:
                    ambiguities.append(f"Requested cash reserve {_fmt(pct)} is below the 10% default; please confirm on the review screen.")
            # Check for USD / dollar exposure
            elif any(term in ctx_lower for term in ["dollar", "usd", "$", "us t-bill"]):
                currency_exposure["USD"] = pct
            # Check for EUR / euro exposure
            elif any(term in ctx_lower for term in ["euro", "eur", "€"]):
                currency_exposure["EUR"] = pct

    max_issuer_weight = _resolve_cap(issuer_caps, DEFAULT_MAX_ISSUER, "issuer", ambiguities, confidence, "max_issuer_weight")
    max_single_asset_weight = _resolve_cap(asset_caps, DEFAULT_MAX_SINGLE_ASSET, "single-asset", ambiguities, confidence,
                                           "max_single_asset_weight")

    # Step 4: a partial split leaves the rest in the vault currency (USD by
    # default; EUR when the dollar share is the one given) - "a third in euro"
    # means two thirds in dollars, not "drop every dollar asset".
    given = sum(currency_exposure.values())
    if currency_exposure and given < 0.999:
        rest = round(1.0 - given, 4)
        if "USD" not in currency_exposure:
            currency_exposure["USD"] = rest
        elif "EUR" not in currency_exposure and any(t in text_lower for t in ["euro", "eur", "€"]):
            currency_exposure["EUR"] = rest
    # Fallback for currency exposure if not parsed
    if currency_exposure:
        confidence["currency_exposure"] = 0.95
    else:
        # Default to USD if no currency mentioned
        if not any(curr in text_lower for curr in ["euro", "eur", "gbp"]):
            currency_exposure["USD"] = 1.00
            confidence["currency_exposure"] = 0.40 if is_vague else 0.70
            ambiguities.append("No explicit currency distribution specified; defaulted to USD.")
        else:
            currency_exposure["EUR"] = 1.00
            confidence["currency_exposure"] = 0.40 if is_vague else 0.70

    # Fallback for liquidity sleeve if not explicitly extracted
    if not has_explicit_liquidity:
        if any(term in text_lower for term in ["runway", "safe", "treasury"]):
            min_liquidity_sleeve = 0.10
            confidence["min_liquidity_sleeve"] = 0.85
        else:
            min_liquidity_sleeve = 0.10  # conservative floor >= 0.05
            confidence["min_liquidity_sleeve"] = 0.50 if is_vague else 0.70
            if is_vague:
                ambiguities.append("No explicit liquidity floor requested; defaulted to 10% instant liquidity sleeve.")

    # 3. Determine Risk Tolerance
    if any(term in text_lower for term in ["safe", "runway", "capital preservation", "conservative", "t-bill"]):
        risk_tolerance = "conservative"
        confidence["risk_tolerance"] = 0.95
    elif any(term in text_lower for term in ["equities", "equity", "tech", "growth", "high yield", "aggressive"]):
        risk_tolerance = "growth"
        confidence["risk_tolerance"] = 0.85
    elif any(term in text_lower for term in ["moderate", "balanced"]):
        risk_tolerance = "moderate"
        confidence["risk_tolerance"] = 0.85
    else:
        # Conservative fallback for ambiguous requests
        risk_tolerance = "conservative"
        confidence["risk_tolerance"] = 0.45 if is_vague else 0.70
        ambiguities.append("Ambiguous risk orientation; defaulted to conservative risk tolerance.")

    # 4. Objectives extraction
    objectives: List[str] = []
    if "runway" in text_lower:
        objectives.append("preserve capital and extend runway")
    if "payroll" in text_lower:
        objectives.append("match operational payroll obligations")
    if "liquidity" in text_lower or has_explicit_liquidity:
        objectives.append("maintain instant liquidity reserve")
    if "t-bill" in text_lower or "treasury" in text_lower:
        objectives.append("earn sovereign t-bill yield")
    if "yield" in text_lower:
        objectives.append("generate sustainable yield")
    if "equities" in text_lower or "tech" in text_lower:
        objectives.append("equity growth exposure")

    if not objectives:
        objectives.append("capital preservation and short-term liquidity")
        confidence["objectives"] = 0.40
    else:
        confidence["objectives"] = 0.90 if not is_vague else 0.55

    # 5. Exclusions
    exclusions: List[str] = []
    if risk_tolerance == "conservative":
        exclusions.extend(["equity", "illiquid"])
    if "no crypto" in text_lower:
        exclusions.append("crypto")
    if "no lending" in text_lower:
        exclusions.append("lending")

    # 6. Yield orientation
    if any(term in text_lower for term in ["high yield", "maximize"]):
        target_yield_orientation = "maximize"
    elif any(term in text_lower for term in ["safe", "runway", "capital preservation"]):
        target_yield_orientation = "secondary"
    else:
        target_yield_orientation = "secondary" if is_vague else "none"

    # Liquidity horizon hours
    liquidity_horizon_hours = 24
    confidence["liquidity_horizon_hours"] = 0.50 if is_vague else 0.90
    if is_vague:
        ambiguities.append("Prompt is brief; defaulted liquidity horizon to 24 hours.")

    return PolicyDraft(
        objectives=objectives,
        currency_exposure=currency_exposure,
        liquidity_horizon_hours=liquidity_horizon_hours,
        max_issuer_weight=max_issuer_weight,
        max_single_asset_weight=max_single_asset_weight,
        risk_tolerance=risk_tolerance,
        exclusions=exclusions,
        min_liquidity_sleeve=min_liquidity_sleeve,
        target_yield_orientation=target_yield_orientation,
        investor_class=investor_class,
        jurisdiction=jurisdiction,
        confidence=confidence,
        ambiguities=ambiguities,
    )
