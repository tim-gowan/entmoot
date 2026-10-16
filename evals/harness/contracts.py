"""Shared failure type for local campaign input and accounting checks."""


class ContractError(ValueError):
    """An input is invalid or its source, tool or observation boundary differs."""
