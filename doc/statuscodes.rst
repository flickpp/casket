Status Codes
----------------

Casket uses the following HTTP status codes.
This list is exhaustive.

500 Internal Server Error
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

* Casket catches a Python exception


503 - Service Busy
~~~~~~~~~~~~~~~~~~~~~~~

Casket worker has reached ``CASKET_MAX_REQUESTS`` limit.
See :ref:`config-max-requests`.

