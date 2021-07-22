import unittest
import tinychain as tc

from testutils import PORT, start_host


CONSERVED = tc.Number(20)


class Balance(tc.Cluster):
    __uri__ = tc.URI("/app/balance")

    def _configure(self):
        self.weight = tc.chain.Sync(tc.UInt(10))


class Left(Balance):
    __uri__ = f"http://127.0.0.1:{PORT}" + tc.uri(Balance) + "/left"

    @tc.post_method
    def weigh(self, txn, weight: tc.Number):
        right = tc.use(Right)

        txn.total = CONSERVED
        txn.update = tc.After(
            self.weight.set(weight),
            right.weigh({"weight": (txn.total - weight)}))

        return tc.If(self.weight == weight, None, txn.update)


class Right(Balance):
    __uri__ = f"http://127.0.0.1:{PORT + 1}" + tc.uri(Balance) + "/right"

    @tc.post_method
    def weigh(self, txn, weight: tc.Number):
        left = tc.use(Left)

        txn.total = CONSERVED
        txn.update = tc.After(
            self.weight.set(weight),
            left.weigh({"weight": (txn.total - weight)}))

        return tc.If(self.weight == weight, None, txn.update)


class InteractionTests(unittest.TestCase):
    def testUpdate(self):
        left = start_host("test_multi_host_left", [Left])
        right = start_host("test_multi_host_right", [Right])

        left.post("/app/balance/left/weigh", {"weight": 5})

        self.assertEqual(left.get("/app/balance/left/weight"), 5)
        self.assertEqual(right.get("/app/balance/right/weight"), 15)


if __name__ == "__main__":
    unittest.main()
