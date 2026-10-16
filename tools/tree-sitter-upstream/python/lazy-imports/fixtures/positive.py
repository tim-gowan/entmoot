import sys

lazy = 41
lazy import fractions as ratios
lazy from decimal import Decimal

assert lazy + 1 == 42
assert "fractions" not in sys.modules
assert "decimal" not in sys.modules


def fraction():
    return ratios.Fraction(1, 2)


assert str(fraction()) == "1/2"
assert "fractions" in sys.modules
assert Decimal("0.5") == Decimal(1) / 2


def identifier(lazy):
    return lazy


assert identifier(7) == 7
