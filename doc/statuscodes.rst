Status Codes
----------------

Casket uses the following HTTP status codes.
This list is exhaustive.

500 - Internal Server Error
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Casket catches a Python exception.

.. _status-codes-503:

503 - Service Busy
~~~~~~~~~~~~~~~~~~~~~~~

Casket worker has reached ``CASKET_MAX_REQUESTS`` limit.
See :ref:`config-max-requests`.


.. _status-codes-504:

504 - Gateway Timeout
~~~~~~~~~~~~~~~~~~~~~~~~

Casket worker has reached ``CASKET_PYTHON_CODE_GATEWAY_TIMEOUT`` limit.
See :ref:`config-python-code-gateway-timeout`.


.. _status-codes-408:

408 - Request Timeout
~~~~~~~~~~~~~~~~~~~~~~~~~

The number of seconds to wait for a request to arrive after we start
reading. This includes *both* header and body.

The start time is when we receive the first byte - after time T,
if we do not have header and body then we send back
408 - Request Timeout.

Time T is configurable with :ref:`config-request-read-timeout`.

Example (client code):

.. code-block:: python

   from socket import socket
   from os import urandom

   # A header with Content-Length 500
   HEADER = b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 500\r\n\r\n"

   sock = socket(("127.0.0.1", 8080))
   sock.send(HEADER)

   # Only send 499 out of 500 expected body bytes
   sock.send(urandom(499))

   # wait to receive some data and print each line
   # this will print a 408 Request Timeout Response
   for line in sock.recv(1024).split(b'\n'):
       print(line.strip(b'\r'))


The output:

.. code-block::

   > HTTP/1.1 408 Request Timeout
   > Server: Casket
   > Connection: Close
