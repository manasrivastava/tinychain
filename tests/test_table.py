import random
import tinychain as tc
import unittest

from num2words import num2words
from testutils import start_host, PORT


ENDPOINT = "/transact/hypothetical"
SCHEMA = tc.Table.Schema(
    [tc.Column("name", tc.String, 512)],
    [tc.Column("views", tc.UInt)],
    {"views": ["views", "name"]})


class TableTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.host = start_host("test_table")

    def testCreate(self):
        cxt = tc.Context()
        cxt.table = tc.Table(SCHEMA)
        cxt.result = tc.After(cxt.table.insert(("name",), (0,)), cxt.table.count())

        count = self.host.post(ENDPOINT, cxt)
        self.assertEqual(count, 1)

    def testInsert(self):
        for x in range(0, 100, 10):
            keys = list(range(x))
            random.shuffle(keys)

            cxt = tc.Context()
            cxt.table = tc.Table(SCHEMA)
            cxt.inserts = [
                cxt.table.insert((num2words(i),), (i,))
                for i in keys]

            cxt.result = tc.After(cxt.inserts, cxt.table.count())

            result = self.host.post(ENDPOINT, cxt)
            self.assertEqual(result, x)

    def testSlice(self):
        count = 50
        values = [(v,) for v in range(count)]
        keys = [(num2words(i),) for i in range(count)]

        cxt = tc.Context()
        cxt.table = tc.Table(SCHEMA)
        cxt.inserts = [cxt.table.insert(k, v) for k, v in zip(keys, values)]
        cxt.result = tc.After(cxt.inserts, cxt.table.where(name="one"))

        result = self.host.post(ENDPOINT, cxt)
        self.assertEqual(result, expected([["one", 1]]))

    def testSliceAuxiliaryIndex(self):
        count = 50
        values = [(v,) for v in range(count)]
        keys = [(num2words(i),) for i in range(count)]

        cxt = tc.Context()
        cxt.table = tc.Table(SCHEMA)
        cxt.inserts = [cxt.table.insert(k, v) for k, v in zip(keys, values)]
        cxt.result = tc.After(cxt.inserts, cxt.table.where(views=slice(10, 20)))

        result = self.host.post(ENDPOINT, cxt)
        self.assertEqual(result, expected(list([[num2words(i), i] for i in range(10, 20)])))


class Persistent(tc.Cluster):
    __uri__ = tc.URI(f"http://127.0.0.1:{PORT}/test/table")

    def _configure(self):
        self.table = tc.Chain.Block(tc.Table(SCHEMA))


class PersistenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        hosts = []
        for i in range(3):
            port = PORT + i
            host_uri = f"http://127.0.0.1:{port}" + tc.uri(Persistent).path()
            host = start_host(f"test_replication_{i}", [Persistent], host_uri=tc.URI(host_uri))
            hosts.append(host)

        cls.hosts = hosts

    def testInsert(self):
        row1 = ["one", 1]
        row2 = ["two", 2]

        self.hosts[0].put("/test/table/table", ["one"], [1])

        for host in self.hosts:
            actual = host.get("/test/table/table", ["one"])
            self.assertEqual(actual, row1)

    @classmethod
    def tearDownClass(cls):
        for host in cls.hosts:
            host.stop()


def expected(rows):
    return {str(tc.uri(tc.Table)): [tc.to_json(SCHEMA), rows]}


if __name__ == "__main__":
    unittest.main()

