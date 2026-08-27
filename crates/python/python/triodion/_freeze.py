from __future__ import annotations

import typing

if typing.TYPE_CHECKING:
    from typing_extensions import Unpack

    from . import _spec


async def async_freeze(
    datatype: str | typing.Sequence[str],
    **kwargs: Unpack[_spec.TriodionCliArgs],
) -> dict[str, int] | None:
    """asynchronously collect data and save to files

    see triodion.parse_kwargs() for descriptions of arguments
    """

    from . import _triodion_rust  # type: ignore
    from . import _args

    if isinstance(datatype, str):
        datatypes = [datatype]
    elif isinstance(datatype, typing.Sequence):
        # The annotation says Sequence[str], but the check was `list`, so a tuple
        # -- the natural shape for a fixed dataset list -- raised. Passing several
        # log-shaped datatypes in one call is what triggers the coalesced
        # MultiDatatype::LogEvents path, so this must accept any sequence.
        datatypes = list(datatype)
    else:
        raise Exception('invalid format for datatype(s)')

    cli_args = _args.parse_cli_args(**kwargs)
    return await _triodion_rust._freeze(datatypes, **cli_args)  # type: ignore


def freeze(
    datatype: str | typing.Sequence[str],
    **kwargs: Unpack[_spec.TriodionCliArgs],
) -> dict[str, int] | None:
    """collect data and save to files"""

    from . import _run

    return _run.run_coroutine(lambda: async_freeze(datatype, **kwargs))

