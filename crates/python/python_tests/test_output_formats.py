import tempfile

import pytest

import triodion
import polars as pl

# Every test in this module reaches out to a real endpoint. See conftest.py:
# without one configured these are skipped rather than failed.
pytestmark = pytest.mark.rpc


queries = [
    {
        'datatype': ['blocks'],
        'start_block': 17000000,
        'end_block': 17000100,
    }
]

file_formats = [
    ['parquet', pl.read_parquet],
    ['csv', pl.read_csv],
    # ['avro', pl.read_avro],
    ['json', pl.read_json],
]


@pytest.mark.parametrize('query', queries)
@pytest.mark.parametrize('format', file_formats)
def test_file_output(query, format):
    extension, reader = format
    output_dir = tempfile.mkdtemp()
    if extension != 'parquet':
        query = dict(query, **{extension: True})
    result = triodion.freeze(output_dir=output_dir, **query)
    for datatype in query['datatype']:
        path = result['paths'][datatype]
        assert isinstance(path, list) and len(path) == 1
        path = path[0]
        df_freeze = reader(path)
        query_without_datatype = dict(query)
        del query_without_datatype['datatype']
        df_collect = triodion.collect(datatype, **query_without_datatype)
        # `frame_equal` was removed in polars 0.20 and renamed to `equals`;
        # the old call raised AttributeError on every modern polars.
        assert df_freeze.equals(df_collect)


python_formats = [
    ['polars', pl.DataFrame],
    ['list', list],
    ['dict', dict],
]


@pytest.mark.parametrize('query', queries)
@pytest.mark.parametrize('format', python_formats)
def test_python_output_formats(query, format):
    # Two bugs here before. The function was named
    # `python_output_python_formats`, without the `test_` prefix, so pytest
    # never collected it. And `python_formats` held plain strings while the
    # body unpacked each into two names, so it would have raised ValueError
    # the moment it did run. Each entry now carries the expected type.
    output_format, output_type = format
    query = dict(query)
    datatype = query.pop('datatype')[0]
    df = triodion.collect(datatype, output_format=output_format, **query)
    assert isinstance(df, output_type)

