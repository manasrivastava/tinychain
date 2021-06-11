from .reflect import Method, MethodStub, Delete, Get, Post, Put


def get_method(method):
    """Annotation for a callable method specifying that it is a GET method."""
    return MethodStub(Method.Get, method)


def put_method(method):
    """Annotation for a callable method specifying that it is a PUT method."""
    return MethodStub(Method.Put, method)


def post_method(method):
    """Annotation for a callable method specifying that it is a POST method."""
    return MethodStub(Method.Post, method)


def delete_method(method):
    """Annotation for a callable method specifying that it is a DELETE method."""
    return MethodStub(Method.Delete, method)


def get_op(op):
    """Annotation for a callable function specifying that it is a GET :class:`Op`."""
    return Get(op)


def put_op(op):
    """Annotation for a callable function specifying that it is a PUT :class:`Op`."""
    return Put(op)


def post_op(op):
    """Annotation for a callable function specifying that it is a POST :class:`Op`."""
    return Post(op)


def delete_op(op):
    """Annotation for a callable function specifying that it is a DELETE :class:`Op`."""
    return Delete(op)
