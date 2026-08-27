from __future__ import annotations

import typing

if typing.TYPE_CHECKING:
    from typing import Any, Callable, Coroutine, TypeVar

    T = TypeVar('T')


def run_coroutine(factory: Callable[[], Coroutine[Any, Any, T]]) -> T:
    """Run one of triodion's async calls from synchronous code.

    Takes a factory rather than a coroutine, because a coroutine can only be
    awaited once.

    The previous implementation, duplicated in both `collect` and `freeze`,
    built a single coroutine and then re-ran that same object inside an
    `except RuntimeError`. Two things were wrong with it:

    - The `except` was there for the case where an event loop is already
      running, but it also caught any `RuntimeError` raised BY the call. The
      retry then failed with "cannot reuse already awaited coroutine", which
      replaced the real message. Every `PyRuntimeError` the Rust extension
      returns is a `RuntimeError`, so omitting `--rpc` reported "cannot reuse
      already awaited coroutine" instead of "must provide --rpc or setup MESC
      or set ETH_RPC_URL".
    - The event loop it created with `asyncio.new_event_loop()` was never
      closed, so every call leaked a loop and its file descriptors.

    Choosing the path by probing for a running loop, instead of by catching an
    exception from the call, leaves errors raised by the call untouched.
    """
    import asyncio

    try:
        asyncio.get_running_loop()
    except RuntimeError:
        # Nothing is running on this thread, so drive the coroutine here.
        # `asyncio.run` creates a loop, runs to completion, and closes it.
        return asyncio.run(factory())

    # A loop is already running on this thread -- a notebook, or an async web
    # handler. Re-entering it is not allowed, so run on a private loop in a
    # worker thread and block until it finishes.
    import concurrent.futures

    with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
        return executor.submit(lambda: asyncio.run(factory())).result()
