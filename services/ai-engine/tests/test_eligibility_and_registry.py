"""Audit A3 / decision D-B: the on-chain registry is the asset universe, and
eligibility = issuer rules (on chain) AND Constella's class policy."""
import pytest
from stellar_sdk import Address

from src import eligibility
from src.models import PolicyDraft, SleeveType
from src.registry_sync import RegistryError, candidates_from_entries
from src.retriever import AssetRegistry, retrieve_candidates
from tests.fixtures import (BLEND_FIXED_V2_POOL, EURC_MAINNET, USDC_MAINNET, entry, fixture_entries,
                            fixture_registry)


def ids(policy):
    return {c.asset_id for c in retrieve_candidates(policy, fixture_registry())}


def test_retail_gets_permissionless_assets_only_even_where_the_issuer_allows_kyc():
    # Franklin's own rules allow non-US (and US) retail with KYC; Constella's
    # policy (D-B) does not offer gated assets to retail until Phase 4.
    for jur in ["EU", "IN", "non-US", "US"]:
        got = ids(PolicyDraft(jurisdiction=jur, investor_class="retail", currency_exposure={}))
        assert got == {"USDC", "EURC", "BLEND_USDC_SUPPLY"}, (jur, got)


def test_professionals_get_gated_assets_where_the_issuer_allows():
    eu = ids(PolicyDraft(jurisdiction="EU", investor_class="professional", currency_exposure={}))
    assert {"FRANKLIN_BENJI", "ONDO_USDY", "SPIKO_USD", "SPIKO_EUR", "JH_JQQQ"} <= eu
    us = ids(PolicyDraft(jurisdiction="US", investor_class="professional", currency_exposure={}))
    assert "FRANKLIN_BENJI" in us
    assert not {"ONDO_USDY", "SPIKO_USD", "SPIKO_EUR", "JH_JQQQ"} & us, "the issuers block US persons"


def test_issuer_class_requirements_are_enforced():
    reg = fixture_registry()
    jqqq = reg.get_asset("JH_JQQQ")
    assert eligibility.check(jqqq, "EU", "professional")[0]
    assert eligibility.check(jqqq, "EU", "institutional")[0]
    ok, why = eligibility.check(jqqq, "EU", "retail")
    assert not ok and "retail" in why
    # The issuer's class rule decides on its own: a QIB-only product (e.g.
    # restricted private credit) is refused to a professional, allowed to an
    # institution - D-B's retail rule is not involved here.
    qib, _ = candidates_from_entries([entry("JH_JQQQ", "X", 5, USDC_MAINNET, permissionless=False, classes=["qib"])])
    ok, why = eligibility.check(qib[0], "EU", "professional")
    assert not ok and "qib" in why
    assert eligibility.check(qib[0], "EU", "institutional")[0]


def test_sanctioned_jurisdictions_get_nothing():
    for jur in ["KP", "ir", " CU "]:
        for cls in ["retail", "professional", "institutional"]:
            assert ids(PolicyDraft(jurisdiction=jur.strip(), investor_class=cls, currency_exposure={})) == set()


def test_inactive_assets_are_never_proposed():
    entries = fixture_entries()
    for e in entries:
        if e["asset_id"] == "BLEND_USDC_SUPPLY":
            e["is_active"] = False
    reg = AssetRegistry.from_entries(entries)
    got = {c.asset_id for c in retrieve_candidates(PolicyDraft(jurisdiction="EU", investor_class="retail", currency_exposure={}), reg)}
    assert "BLEND_USDC_SUPPLY" not in got
    assert "not active" in reg.eligibility("EU", "retail")["BLEND_USDC_SUPPLY"]


def test_an_issuer_allow_list_fails_closed_for_vague_jurisdictions():
    a, _ = candidates_from_entries([entry("SPIKO_EUR", "SPIKO", 2, USDC_MAINNET, permissionless=False, eligible=["FR", "DE"])])
    asset = a[0]
    assert eligibility.check(asset, "FR", "professional")[0]
    assert not eligibility.check(asset, "non-US", "professional")[0], "generic 'non-US' cannot prove FR/DE"
    assert not eligibility.check(asset, "IN", "professional")[0]


def test_the_chain_is_the_only_source_of_assets_and_addresses():
    reg = fixture_registry()
    assert reg.get_asset("USDC").contract_address == USDC_MAINNET
    assert reg.get_asset("EURC").contract_address == EURC_MAINNET, "the verified Circle EURC SAC"
    assert reg.get_asset("BLEND_USDC_SUPPLY").contract_address == BLEND_FIXED_V2_POOL
    assert reg.get_asset("BLEND_USDC_SUPPLY").sleeve_type == SleeveType.LENDING
    # A described asset that is not on chain does not exist.
    only_usdc = AssetRegistry.from_entries([e for e in fixture_entries() if e["asset_id"] == "USDC"])
    assert [a.asset_id for a in only_usdc.list_assets()] == ["USDC"]


def test_on_chain_assets_without_a_description_are_skipped_not_invented():
    cands, skipped = candidates_from_entries(fixture_entries() + [entry("MYSTERY", "X", 0, USDC_MAINNET)])
    assert "MYSTERY" in skipped and all(c.asset_id != "MYSTERY" for c in cands)


def test_unknown_sleeve_codes_are_refused():
    with pytest.raises(RegistryError):
        candidates_from_entries([entry("USDC", "CIRCLE", 9, USDC_MAINNET)])


def test_production_yields_are_unknown_not_invented():
    reg = AssetRegistry.from_entries(fixture_entries())
    assert all(a.current_yield_pct == 0.0 for a in reg.list_assets())


def test_unknown_investor_classes_are_refused():
    with pytest.raises(ValueError):
        eligibility.check(fixture_registry().get_asset("USDC"), "EU", "whale")


def test_addresses_decode_from_sdk_address_objects():
    cands, _ = candidates_from_entries([entry("USDC", "CIRCLE", 0, USDC_MAINNET)])
    assert isinstance(Address(cands[0].contract_address), Address)
