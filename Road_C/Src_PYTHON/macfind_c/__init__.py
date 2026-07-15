"""MacFind Road_C (Python) — hybrid macOS file search.

Road_C is the *complete hybrid* implementation: the primary search path is a
self-built binary index (numpy-vectorised bitmask pre-filter + fzf scoring,
loaded via ``np.memmap``); when the index is missing or corrupt the engine
degrades to a live ``searchfs()`` scan via ctypes.

See :mod:`macfind_c.engine` for the orchestration and ``README.md`` for the
build/CI story.
"""

__version__ = "0.1.0"

__all__ = ["__version__"]
