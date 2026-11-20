"""Constella price feeder (audit O7).

Pushes prices that Constella derives itself to the NAV oracle's pushed sources,
signing with that source's own feeder key. The first source is the
fundamental EURC/USDC price from the ECB EUR/USD reference rate.
"""
