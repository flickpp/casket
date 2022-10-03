version  0.3 (Upcoming)
~~~~~~~~~~~~~~~~~~~~~~~~~~

* **FEATURE** `Async HTTP`_ requests casket module
* **FEATURE** `Casket-Dev`_ Python script to run locally for development. Colored log filtering.
* **CORE** `Python Code Timeout`_


.. _Async HTTP: https://github.com/flickpp/casket/issues/10
.. _Casket-Dev: https://github.com/flickpp/casket/issues/17
.. _Python Code Timeout: https://github.com/flickpp/casket/issues/7


version 0.2
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

* **FEATURE:** `Casket ndjsonlogger`_ exposed to python. Integrate with trace context.
* **DISTRIBUTION:** `Docker image`_ with avaliability on dockerhub.
* **CORE:** `SIGINT (Ctrl-C)`_ graceful shutdown. Complete currently processing requests before ending process.
* **CORE** `Request Read Timeout`_. Timeout a requests after time T, if we still haven't received the complete header and body for said request.

Bug Fixes:

* `Content-Length too large`_ does not cause out of bounds array read
* `Stream EOF handled gracefully`_
  

.. _version 0.2: https://github.com/flickpp/casket/issues?q=milestone%3A%22release+0.2%22+
.. _Casket ndjsonlogger: https://github.com/flickpp/casket/issues/4
.. _Docker image: https://github.com/flickpp/casket/issues/1
.. _SIGINT (Ctrl-C): https://github.com/flickpp/casket/issues/5
.. _Request Read Timeout: https://github.com/flickpp/casket/issues/6
.. _Content-Length too large: https://github.com/flickpp/casket/issues/15
.. _Stream EOF handled gracefully: https://github.com/flickpp/casket/issues/14



version 0.1
~~~~~~~~~~~~~~~~~~~~~~~~~

* Multi-Process / Multi-Thread
* Return Python Stacktrace in HTTP body
* Integration with `trace context <https://www.w3.org/TR/trace-context-1/>`_
* ``503 Server Too Busy`` HTTP response on heavy load
* Distribution with source code
