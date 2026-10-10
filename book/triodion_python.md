# Python

`collect` returns the data as a dataframe. `freeze` writes the data to files, the same as the CLI.

```python
import triodion

# Return the data as a polars DataFrame.
df = triodion.collect('blocks', blocks=['18M:+100'], rpc='http://localhost:8545')

# Write the data to parquet files in ./data.
triodion.freeze('erc20_transfers', blocks=['18M:+100'], output_dir='data')
```

Each keyword argument is a CLI flag:

- Write the flag name with underscores in place of dashes. For example, `--max-retries` becomes `max_retries`.
- A flag that turns a feature off uses the name of the feature. For example, `--no-multicall` becomes `multicall=False`.
- A keyword that you do not give has the CLI default. A keyword with the value `None` also has the CLI default.
- An unknown keyword, or a value of the wrong type, raises `TypeError`. The message names the keyword.

If you do not give `rpc`, triodion uses the `ETH_RPC_URL` environment variable.
