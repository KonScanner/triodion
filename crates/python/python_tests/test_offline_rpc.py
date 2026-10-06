"""Tests that drive real collection against a local, replayed JSON-RPC endpoint.

Every `rpc`-marked test is skipped in CI unless an `ETH_RPC_URL` secret is
configured, so without this module nothing in CI moves a DataFrame from the
Rust extension into Python. That hand-off is the most version-sensitive part
of the package: `pyo3-polars` passes each column to the installed Python
polars through `Series._import(<pointer>)`, an FFI contract between two
separately built copies of polars. A mismatch there is invisible to every
Rust-side check.

The endpoint here answers from `fixtures/mainnet_blocks_21000000.json`, two
mainnet blocks recorded from a public node with their `transactions` hash
lists emptied to keep the file small. The `blocks` dataset reads only header
fields and withdrawals, so the emptied list changes none of its columns.
"""

from __future__ import annotations

import json
import pathlib
import threading
import typing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import polars as pl
import pytest

import triodion

FIXTURE = pathlib.Path(__file__).parent / 'fixtures' / 'mainnet_blocks_21000000.json'
BLOCKS = ['21_000_000:21_000_002']


def _answer(call: dict[str, typing.Any], recorded: dict[str, typing.Any]) -> dict[str, typing.Any]:
    method = call.get('method')
    result: typing.Any = None
    if method == 'eth_chainId':
        result = recorded['chain_id']
    elif method == 'eth_getBlockByNumber':
        result = recorded['blocks'].get(call['params'][0])
    else:
        return {
            'jsonrpc': '2.0',
            'id': call.get('id'),
            'error': {'code': -32601, 'message': f'not replayed: {method}'},
        }
    return {'jsonrpc': '2.0', 'id': call.get('id'), 'result': result}


@pytest.fixture(scope='module')
def rpc_url() -> typing.Iterator[str]:
    recorded = json.loads(FIXTURE.read_text())

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            if isinstance(body, list):
                reply: typing.Any = [_answer(call, recorded) for call in body]
            else:
                reply = _answer(body, recorded)
            payload = json.dumps(reply).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, format: str, *args: typing.Any) -> None:
            pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f'http://127.0.0.1:{server.server_port}'
    finally:
        server.shutdown()
        server.server_close()


def test_collect_returns_a_polars_frame_with_the_declared_dtypes(rpc_url):
    df = triodion.collect('blocks', blocks=BLOCKS, rpc=rpc_url)

    assert isinstance(df, pl.DataFrame)
    assert df.height == 2
    assert df.schema['block_hash'] == pl.Binary
    assert df.schema['block_number'] == pl.UInt32
    assert df.schema['gas_used'] == pl.UInt64
    assert df.schema['base_fee_per_gas'] == pl.UInt64
    assert df.schema['chain_id'] == pl.UInt64

    assert df['block_number'].to_list() == [21_000_000, 21_000_001]
    assert df['gas_used'].to_list() == [0xD4E774, 0xD8E312]
    assert df['base_fee_per_gas'].to_list() == [0x43C4BCEC1, 0x432D5A7CE]
    assert df['chain_id'].to_list() == [1, 1]

    recorded = json.loads(FIXTURE.read_text())['blocks']
    expected_hash = bytes.fromhex(recorded['0x1406f40']['hash'][2:])
    assert df['block_hash'][0] == expected_hash


@pytest.mark.parametrize(
    ('output_format', 'output_type'),
    [('polars', pl.DataFrame), ('list', list), ('dict', dict)],
)
def test_every_python_output_format_survives_the_hand_off(rpc_url, output_format, output_type):
    result = triodion.collect('blocks', output_format=output_format, blocks=BLOCKS, rpc=rpc_url)
    assert isinstance(result, output_type)


def test_freeze_writes_parquet_that_reads_back_equal_to_collect(rpc_url, tmp_path):
    result = triodion.freeze('blocks', blocks=BLOCKS, rpc=rpc_url, output_dir=str(tmp_path))
    assert result['n_errored'] == 0

    # The freeze summary carries counts only, not output paths.
    (path,) = tmp_path.glob('*.parquet')
    frozen = pl.read_parquet(path)
    collected = triodion.collect('blocks', blocks=BLOCKS, rpc=rpc_url)
    assert frozen.equals(collected)
