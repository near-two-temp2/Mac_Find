"""mac_find_a — Road_A (searchfs, no index) macOS fast file search.

Public surface:
    searchfs_engine.search(term, opts, ...)   -> streaming path iterator
    searchfs_engine.SearchOptions             -> query options dataclass
    searchfs_engine.searchfs_available()      -> bool (True on macOS)
"""

from . import searchfs_engine  # noqa: F401
from .searchfs_engine import SearchOptions, search, searchfs_available  # noqa: F401

__all__ = ["searchfs_engine", "SearchOptions", "search", "searchfs_available"]
__version__ = "0.1.0"
