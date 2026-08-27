"""Shared pytest configuration for the triodion Python tests.

Most tests here drive `triodion.collect` / `triodion.freeze` against real
chain data, so they need an Ethereum endpoint from `--rpc`, MESC, or
`ETH_RPC_URL`. Without one they all fail on the same missing-endpoint error,
which says nothing about the code under test.

Rather than skip the whole suite, those tests carry the `rpc` marker and are
skipped only when no endpoint is configured. Tests without the marker run
everywhere, so CI still proves that the wheel builds, the extension imports,
and the error paths behave. Set `ETH_RPC_URL` and the marked tests run too,
locally and in CI, with no further change.
"""

import os

import pytest


def _endpoint_configured() -> bool:
    return bool(os.environ.get('ETH_RPC_URL') or os.environ.get('MESC_MODE'))


def pytest_collection_modifyitems(config, items):
    if _endpoint_configured():
        return
    skip_rpc = pytest.mark.skip(
        reason='needs an Ethereum endpoint: set ETH_RPC_URL (or configure MESC)'
    )
    for item in items:
        if 'rpc' in item.keywords:
            item.add_marker(skip_rpc)
