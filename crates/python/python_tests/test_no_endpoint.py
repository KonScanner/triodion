"""Tests that run without an Ethereum endpoint.

Everything else in this directory needs live chain data and is skipped when no
endpoint is configured. These do not: they cover the package surface and the
error path, so a CI run with no `ETH_RPC_URL` still proves that the wheel
builds, the extension module imports, and a missing endpoint is reported
usefully rather than as an opaque crash.
"""

import asyncio

import pytest

import triodion


def test_package_exports_its_public_surface():
    for name in ('collect', 'freeze', 'async_collect', 'async_freeze'):
        assert hasattr(triodion, name), name
    assert isinstance(triodion.__version__, str)


@pytest.mark.parametrize('call', ['collect', 'freeze'])
def test_a_missing_endpoint_reports_what_is_missing(monkeypatch, call, tmp_path):
    """A missing endpoint must name itself, not surface as a crash.

    Two separate defects used to hide this message.

    The Rust adapters called `panic!` / `.expect()` on the parse error, and a
    panic crosses the pyo3 boundary as
    `RustPanic: rust future panicked: unknown error`, dropping the text.

    Then the synchronous wrappers built one coroutine, ran it, and re-ran that
    same object inside `except RuntimeError`. Because the extension reports
    failures as `PyRuntimeError`, that `except` caught the real error and the
    retry replaced it with "cannot reuse already awaited coroutine".
    """
    monkeypatch.delenv('ETH_RPC_URL', raising=False)
    monkeypatch.delenv('MESC_MODE', raising=False)

    kwargs = {'blocks': ['17_000_000:17_000_010']}
    if call == 'freeze':
        kwargs['output_dir'] = str(tmp_path)

    with pytest.raises(RuntimeError) as excinfo:
        getattr(triodion, call)('blocks', **kwargs)

    message = str(excinfo.value)
    assert 'must provide --rpc' in message, message
    assert 'cannot reuse already awaited coroutine' not in message, message


def test_the_synchronous_wrapper_works_inside_a_running_loop(monkeypatch, tmp_path):
    """`collect` must be callable from inside an already-running event loop.

    That is the notebook case. The wrapper detects the running loop and drives
    the coroutine on a private loop in a worker thread. Reaching the endpoint
    error rather than "this event loop is already running" is what shows the
    dispatch worked.
    """
    monkeypatch.delenv('ETH_RPC_URL', raising=False)
    monkeypatch.delenv('MESC_MODE', raising=False)

    async def main():
        with pytest.raises(RuntimeError) as excinfo:
            triodion.collect('blocks', blocks=['17_000_000:17_000_010'])
        return str(excinfo.value)

    message = asyncio.run(main())
    assert 'must provide --rpc' in message, message
