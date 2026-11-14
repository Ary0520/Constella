"""
Asset Registry and Candidate Retrieval Module for Constella AI Thesis Engine.
Enforces the Closed Universe Rule: zero hallucinated or unregistered assets.
"""

from typing import List, Optional
from src.models import AssetCandidate, PolicyDraft, SleeveType


class AssetRegistry:
    """
    In-memory verified registry of Stellar RWA and cash assets.
    Mirrors on-chain registry contract constraints.
    """

    DEFAULT_ASSETS = [
        AssetCandidate(
            asset_id="USDC",
            display_name="USD Coin",
            issuer="Circle",
            sleeve_type=SleeveType.CASH_STABLE,
            currency="USD",
            liquidity_tier=0,
            permissionless=True,
            requires_professional=False,
            requires_institutional=False,
            blocked_jurisdictions=[],
            contract_address="CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75",
            current_yield_pct=0.0,
            tags=["cash", "stablecoin", "usd", "circle", "instant", "numeraire", "liquidity"],
            description="Circle USD Coin on Stellar. Permissionless instant liquidity numéraire.",
        ),
        AssetCandidate(
            asset_id="EURC",
            display_name="Euro Coin",
            issuer="Circle",
            sleeve_type=SleeveType.CASH_STABLE,
            currency="EUR",
            liquidity_tier=0,
            permissionless=True,
            requires_professional=False,
            requires_institutional=False,
            blocked_jurisdictions=[],
            contract_address="CDTKPW7E2YS77THNKAZHJ4DA2ULBCNXF2722J4ENFBNI4JJ23SSX2X4T",
            current_yield_pct=0.0,
            tags=["cash", "stablecoin", "eur", "circle", "instant", "payroll", "euro", "liquidity"],
            description="Circle Euro Coin on Stellar. Permissionless instant liquidity euro numéraire.",
        ),
        AssetCandidate(
            asset_id="BLEND_USDC_SUPPLY",
            display_name="Blend USDC Supply",
            issuer="Blend",
            sleeve_type=SleeveType.LENDING,
            currency="USD",
            liquidity_tier=1,
            permissionless=True,
            requires_professional=False,
            requires_institutional=False,
            blocked_jurisdictions=[],
            contract_address="CAQSMXQWQG6G33E3G7R6G3PGB4KBL45Q3KRLYV7VNJB663NXZI6VBLEND",
            current_yield_pct=6.50,
            tags=["lending", "defi", "usd", "blend", "yield", "variable"],
            description="Blend protocol USDC lending supply pool on Stellar.",
        ),
        AssetCandidate(
            asset_id="FRANKLIN_BENJI",
            display_name="Franklin Templeton OnChain U.S. Government Money Fund (BENJI)",
            issuer="Franklin Templeton",
            sleeve_type=SleeveType.USD_GOV_FUNDS,
            currency="USD",
            liquidity_tier=1,
            permissionless=False,
            requires_professional=False,
            requires_institutional=False,
            blocked_jurisdictions=[],
            contract_address="CB6N42OQJ3FGB22N47M6P4X6Q6LMBENJI43572KJNK3VNZP6QW4BENJI",
            current_yield_pct=4.90,
            tags=["treasury", "t-bill", "usd", "benji", "franklin", "government", "money-market", "safe", "yield"],
            description="Tokenized US government money market fund; invests in US Treasury bills and repos; daily liquidity.",
        ),
        AssetCandidate(
            asset_id="SPIKO_USD",
            display_name="Spiko USD T-bills",
            issuer="Spiko",
            sleeve_type=SleeveType.USD_GOV_FUNDS,
            currency="USD",
            liquidity_tier=1,
            permissionless=False,
            requires_professional=False,
            requires_institutional=False,
            blocked_jurisdictions=["US"],
            contract_address="CDSPIKOUSD6GB22N47M6P4X6Q6LMSPIKO43572KJNK3VNZP6QW4SPIKO",
            current_yield_pct=4.85,
            tags=["treasury", "t-bill", "usd", "spiko", "sovereign", "yield", "safe"],
            description="Spiko tokenized USD short-term Treasury bills.",
        ),
        AssetCandidate(
            asset_id="SPIKO_EUR",
            display_name="Spiko EUR T-bills",
            issuer="Spiko",
            sleeve_type=SleeveType.EUR_SOVEREIGN,
            currency="EUR",
            liquidity_tier=1,
            permissionless=False,
            requires_professional=False,
            requires_institutional=False,
            blocked_jurisdictions=["US"],
            contract_address="CDSPIKOEUR6GB22N47M6P4X6Q6LMSPIKO43572KJNK3VNZP6QW4SPIKO",
            current_yield_pct=3.25,
            tags=["treasury", "t-bill", "eur", "euro", "spiko", "sovereign", "payroll", "safe", "yield"],
            description="Spiko tokenized EUR short-term sovereign T-bills for euro cash and payroll management.",
        ),
        AssetCandidate(
            asset_id="JH_JQQQ",
            display_name="Janus Henderson Anemoy JQQQ (Nasdaq-100)",
            issuer="Janus Henderson / Centrifuge",
            sleeve_type=SleeveType.EQUITY,
            currency="USD",
            liquidity_tier=2,
            permissionless=False,
            requires_professional=True,
            requires_institutional=False,
            blocked_jurisdictions=["US"],
            contract_address="CAJQQQ6GB22N47M6P4X6Q6LMJQQQ43572KJNK3VNZP6QW4JHJQQQ",
            current_yield_pct=0.0,
            tags=["equity", "tech", "nasdaq", "growth", "janus-henderson", "jqqq", "high-yield"],
            description="Tokenized Nasdaq-100 index fund for non-US professional investors.",
        ),
    ]

    def __init__(self, assets: Optional[List[AssetCandidate]] = None):
        if assets is not None:
            self._assets = {a.asset_id: a for a in assets}
        else:
            self._assets = {a.asset_id: a for a in self.DEFAULT_ASSETS}

    def get_asset(self, asset_id: str) -> Optional[AssetCandidate]:
        return self._assets.get(asset_id)

    def list_assets(self) -> List[AssetCandidate]:
        return list(self._assets.values())

    def filter_by_jurisdiction(self, jurisdiction: str) -> List[AssetCandidate]:
        """
        Exclude assets where the given jurisdiction is explicitly blocked.
        """
        norm_jur = jurisdiction.strip().upper()
        return [
            a for a in self._assets.values()
            if not any(bj.strip().upper() == norm_jur for bj in a.blocked_jurisdictions)
        ]

    def filter_by_investor_class(self, investor_class: str) -> List[AssetCandidate]:
        """
        Filter assets by investor accreditation class.
        - retail: only assets not requiring professional or institutional status (or permissionless)
        - professional: assets not requiring institutional status
        - institutional: all assets
        """
        norm_class = investor_class.strip().lower()
        if norm_class == "retail":
            return [
                a for a in self._assets.values()
                if a.permissionless or (not a.requires_professional and not a.requires_institutional)
            ]
        elif norm_class == "professional":
            return [
                a for a in self._assets.values()
                if not a.requires_institutional
            ]
        else:  # institutional
            return list(self._assets.values())


def retrieve_candidates(policy: PolicyDraft, registry: AssetRegistry) -> List[AssetCandidate]:
    """
    Retrieve candidate assets from the registry matching policy constraints.
    Enforces:
    1. Jurisdiction compatibility (blocked jurisdictions stripped)
    2. Investor class gating (institutional/professional gating)
    3. Exclusions (sleeves or specific asset IDs excluded)
    4. Target currency alignment
    """
    # 1. Filter by jurisdiction compatibility
    eligible = registry.filter_by_jurisdiction(policy.jurisdiction)

    # 2. Filter by investor class eligibility
    norm_class = policy.investor_class.strip().lower()
    if norm_class == "retail":
        eligible = [
            a for a in eligible
            if a.permissionless or (not a.requires_professional and not a.requires_institutional)
        ]
    elif norm_class == "professional":
        eligible = [
            a for a in eligible
            if not a.requires_institutional
        ]
    # institutional allows all eligible assets

    # 3. Filter by exclusions
    exclusions_lower = {e.strip().lower() for e in policy.exclusions}
    eligible = [
        a for a in eligible
        if a.sleeve_type.value.lower() not in exclusions_lower
        and a.sleeve_type.name.lower() not in exclusions_lower
        and a.asset_id.lower() not in exclusions_lower
    ]

    # 4. Target currency matching (if specified in currency_exposure)
    if policy.currency_exposure:
        target_currencies = {c.strip().upper() for c, w in policy.currency_exposure.items() if w > 0}
        # Keep assets matching target currencies; if none match, keep all eligible
        matching = [a for a in eligible if a.currency.upper() in target_currencies]
        if matching:
            eligible = matching

    return eligible
