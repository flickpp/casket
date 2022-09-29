
`version 0.2`_ (upcoming)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

* **FEATURE:** `Casket ndjsonlogger`_ exposed to python. Integrate with trace context.
* **DISTRIBUTION:** `Docker image`_ with avaliability on dockerhub.
* **CORE:** `SIGINT (Ctrl-C)`_ graceful shutdown. Complete currently processing requests before ending process.
* **CORE** `Request Read Timeout`_. Timeout a requests after time T, if we still haven't received the complete header and body for said request.

.. _version 0.2: https://github.com/flickpp/casket/issues?q=milestone%3A%22release+0.2%22+
.. _Casket ndjsonlogger: https://github.com/flickpp/casket/issues/4
.. _Docker image: https://github.com/flickpp/casket/issues/1
.. _SIGINT (Ctrl-C): https://github.com/flickpp/casket/issues/5
.. _Request Read Timeout: https://github.com/flickpp/casket/issues/6



version 0.1
~~~~~~~~~~~~~~~~~~~~~~~~~

* Multi-Process / Multi-Thread
* Return Python Stacktrace in HTTP body
* Integration with `trace context <https://www.w3.org/TR/trace-context-1/>`_
* ``503 Server Too Busy`` HTTP response on heavy load
* Distribution with source code
