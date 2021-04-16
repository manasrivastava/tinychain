"""Data structures responsible for storing a collection of :class:`Value`\s"""

from .ref import OpRef
from .reflect import Meta
from .state import State
from .util import *
from .value import UInt, Nil, Value


class Column(object):
    """
    A column in the schema of a :class:`BTree`.
    """

    def __init__(self, name, dtype, max_size=None):
        self.name = str(name)
        self.dtype = uri(dtype)
        self.max_size = max_size

    def __json__(self):
        if self.max_size is None:
            return to_json((self.name, self.dtype))
        else:
            return to_json((self.name, self.dtype, self.max_size))


class Collection(State, metaclass=Meta):
    """
    Data structure responsible for storing a collection of :class:`Value`\s.
    """

    __uri__ = uri(State) + "/collection"


class BTree(Collection):
    """
    A BTree with a schema of named, :class:`Value`-typed :class:`Column`\s.
    """

    __uri__ = uri(Collection) + "/btree"

    class Schema(object):
        """
        A BTree schema which comprises a tuple of :class:`Column`\s.
        """

        def __init__(self, *columns):
            self.columns = columns

        def __json__(self):
            return to_json(self.columns)

    def __getitem__(self, prefix):
        """
        Return a slice of this BTree containing all keys which begin with the given prefix.
        """

        return BTree(OpRef.Get(uri(self), prefix))

    def count(self):
        """
        Return the number of keys in this BTree.

        To count the number of keys beginning with a specific prefix,
        call `btree[prefix].count()`.
        """

        return UInt(OpRef.Get(uri(self) + "/count"))

    def delete(self):
        """
        Delete the contents of this BTree.
        
        To delete all keys beginning with a specific prefix, call
        `btree[prefix].delete()`.
        """

        return Nil(OpRef.Delete(uri(self)))

    def insert(self, key):
        """
        Insert the given key into this BTree.

        If the key is already present, this is a no-op.
        """

        return Nil(OpRef.Put(uri(self) + "/insert", None, key))

    def reverse(self):
        """
        Return a slice of this BTree with the same range with its keys in reverse order.
        """

        return BTree(OpRef.Get(uri(self), (None, True)))

