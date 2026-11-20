"""ECB euro foreign exchange reference rates (primary, public, no key).

The ECB publishes once per TARGET business day; rates come from the daily
concertation at 14:15 CET/CEST. Weekends and TARGET holidays (1 Jan, Good
Friday, Easter Monday, 1 May, 25-26 Dec) have no new rate, so the oracle
source that carries this price needs a staleness limit covering ~5 days.
"""
from __future__ import annotations

import urllib.request
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from datetime import date, datetime, time
from decimal import Decimal, InvalidOperation
from zoneinfo import ZoneInfo

ECB_DAILY_URL = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml"
_NS = {"e": "http://www.ecb.int/vocabulary/2002-08-01/eurofxref"}
_FRANKFURT = ZoneInfo("Europe/Berlin")
_CONCERTATION = time(14, 15)


class EcbError(ValueError):
    """The ECB document is missing, malformed or lacks the USD rate."""


@dataclass(frozen=True)
class EcbRate:
    day: date
    usd_per_eur: Decimal  # e.g. Decimal("1.1052")

    @property
    def reference_unix(self) -> int:
        """Unix seconds of the concertation (14:15 Frankfurt time) the rate refers to."""
        return int(datetime.combine(self.day, _CONCERTATION, tzinfo=_FRANKFURT).timestamp())


def parse_usd_rate(xml_bytes: bytes) -> EcbRate:
    try:
        root = ET.fromstring(xml_bytes)
    except ET.ParseError as e:
        raise EcbError(f"not XML: {e}") from e
    days = root.findall(".//e:Cube[@time]", _NS)
    if len(days) != 1:
        raise EcbError(f"expected exactly one dated Cube, found {len(days)}")
    day_cube = days[0]
    try:
        day = date.fromisoformat(day_cube.attrib["time"])
    except ValueError as e:
        raise EcbError(f"bad date {day_cube.attrib.get('time')!r}") from e
    usd = [c for c in day_cube.findall("e:Cube", _NS) if c.attrib.get("currency") == "USD"]
    if len(usd) != 1:
        raise EcbError("USD rate missing or duplicated")
    try:
        rate = Decimal(usd[0].attrib["rate"])
    except (KeyError, InvalidOperation) as e:
        raise EcbError("unparseable USD rate") from e
    if not rate.is_finite() or rate <= 0:
        raise EcbError(f"non-positive USD rate {rate}")
    return EcbRate(day=day, usd_per_eur=rate)


def fetch_daily(url: str = ECB_DAILY_URL, timeout: float = 20.0) -> bytes:
    """Fetch the daily XML. `file://` URLs are accepted for offline replays."""
    req = urllib.request.Request(url, headers={"User-Agent": "constella-feeder/0.1"})
    with urllib.request.urlopen(req, timeout=timeout) as resp:  # noqa: S310 (fixed https/file URL)
        return resp.read()
