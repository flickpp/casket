
Configuration
-------------------

Out of the box Casket should comfortably handle thousands of requests per-second.
We have chosen sensible defaults that should be fine in the majority of situations.

All casket configuration is done with environment variables.

CASKET_BIND_ADDR
~~~~~~~~~~~~~~~~~~~~~~~~~

``DEFAULT: 0.0.0.0:8080``

The address on which the server will bind.

Example:

``CASKET_BIND_ADDR=0.0.0.0:9000``

CASKET_NUM_WORKERS
~~~~~~~~~~~~~~~~~~~~~

``DEFAULT: 3``

Casket maintains a proccess pool with each process containing a python interpreter.
This variable controlls how many proccesses to spawn in the pool.

Example:

``CASKET_NUM_WORKERS=5``

CASKET_MAX_CONNECTIONS
~~~~~~~~~~~~~~~~~~~~~~~~~

``DEFAULT: 256``

| The maximum number of open, but potentially idle TCP streams.
| NOTE: This is global and NOT per worker.

If this limit is reached Casket will accept, then immediatly shutdown any
newer TCP streams. Furthermore we log a warning.

.. code-block:: json

   {"level":"warn", "msg": "maximum number of tcp streams exceeded"}

Example:

``CASKET_MAX_CONNECTIONS=64``


.. _config-max-requests:

CASKET_MAX_REQUESTS
~~~~~~~~~~~~~~~~~~~~~~~~

``DEFAULT: 64``

When a Casket *worker* has received N HTTP requests (header and body) it places them
into a queue for python threads to execute. If this queue becomes full Casket will return
an error.

NOTE: This limit is per-worker.

If this limit is reached Casket will send back ``HTTP/1.1 503 Service Busy`` response header.
See :ref:`status-codes-503`.

Example:

``CASKET_MAX_REQUESTS=8``


CASKET_RETURN_STACKTRACE_IN_BODY
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. admonition:: NOTE
   :class: important

   Many Python WSGI frameworks have some internal exception handling mechanism
   and this variable will have no effect.

   `Flask <https://palletsprojects.com/p/flask/>`_ will catch the exception and
   call ``environ['wsgi.errors'].writeline()``. It will not raise the exception
   to Casket. See :ref:`environ-wsgi-errors` for how Casket implements the
   ``writeline`` function.

| ``DEFAULT: 1``
| ``DEFAULT: 0`` (in Casket Docker image)

When a Python exception is caught by Casket, optionally return the stacktrace as HTTP response body.

Casket will always return ``500 Internal Server Error`` if a Python exception is caught.
It does not matter what this value is set to.

Set this value to:

| ``CASKET_RETURN_STACKTRACE_IN_BODY=0`` (feature off)
| ``CASKET_RETURN_STACKTRACE_IN_BODY=1`` (feature on)

Example:

.. code-block:: python
   :linenos:

   # file wsgi.py

   def goose():
       1 / 0

   def hello_world():
       goose()

   def app(environ, start_response):
       hello_world()
       start_response("200 Ok", [("Content-Length", "0")])
       return (b'',)


With this feature ON Casket will set the X-Error header and HTTP response body as so:

.. code-block::

   < HTTP/1.1 500 Internal Server Error
   < Content-Length: 185
   < Content-Type: text/plain; charset=UTF-8
   < X-Error: division by zero
   < X-TraceId: 4c4588ff2a399b64c8393a6ab26bc85d
   < Connection: Keep-Alive
   < Server: Casket

   Traceback (most recent call last):
     File "wsgi.py", line 10, in app
       hello_world()
     File "wsgi.py", line 7, in hello_world
       goose()
     File "wsgi.py", line 4, in goose
       1 / 0



CASKET_LOG_HTTP_RESPONSE
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

``DEFAULT: 1``

Casket will log one line at the info level per-http HTTP request-response pair.
If this feature is turned off then Casket will **not log anything** during a
successful request-response cycle. Casket will *always* log lines the **error**
level.

.. admonition:: NOTE
   :class: important

   If running in production and expecting a lot of garbage traffic you might want to turn this off.

In detail - we actually log one per line per *attempted* HTTP request,
where an attempted request is one or more bytes received over the TCP stream.

If we fail socket I/O or can't parse the HTTP header etc. then we still
log **exactly** one line at the **info** level. This log line is still at
the info level but will have an "error" JSON key in the log line.

Example:

Below we see Content-Length is a bad value.

NOTE: The **info** log level and the **error** key in the JSON.

.. code-block::

   > GET / HTTP/1.1
   > Host: localhost:8090
   > Accept: */*
   > Content-Length:foo

Causing Casket to log, we note the **info** log level and the **error** key in the JSON.

.. code-block:: json

   {"level":"info","ts":"2022-09-28T15:30:08.922795Z","msg":"failed to read http request","error":"Content-Length not uint"}

Set this value to:

| ``CASKET_LOG_HTTP_RESPONSE=0`` (feature off)
| ``CASKET_LOG_HTTP_RESPONSE=1`` (feature on)


CASKET_CTRLC_WAIT_TIME
~~~~~~~~~~~~~~~~~~~~~~~~~

``DEFAULT: 10``

When Casket receives ctrl-c (or SIGINT inside a Docker container) it will finish
processing any active requests, notify client(s) with socket shutdown then exit.

If after time ``CASKET_CTRLC_WAIT_TIME`` there are still active requests then
Casket will exit anyway. The value is given in seconds.

Example:

``CASKET_CTRLC_WAIT_TIME=25``


.. _config-request-read-timeout:

CASKET_REQUEST_READ_TIMEOUT
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

``DEFAULT: 30``

The number of seconds to wait for a request to arrive after we start
reading. This includes *both* header and body.

See :ref:`status-codes-408`.

Example:

``CASKET_REQUEST_READ_TIMEOUT=25``


.. _config-python-code-gateway-timeout:

CASKET_PYTHON_CODE_GATEWAY_TIMEOUT
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

``DEFAULT: 10``

When Python WSGI application starts executing, wait at most time T for a response.
If response is not received send back ``504 Gateway Timeout``.

See also :ref:`status-codes-504`.


Example (code):

.. code-block:: python
   :linenos:

   from time import sleep
   from casket.logger import info
   from flask import Flask

   app = Flask(__name__)

   @app.route('/')
   def hello_world():
       sleep(15)
       info("returning Hello World")
       return 'Hello World!'


If we send a request to this WSGI application - we get the following response.
(The default is after 10 seconds) as stated above.

.. code-block::

   < HTTP/1.1 504 Gateway Timeout
   < Server: Casket
   < Connection: Close


We note that the python code will **continue executing**. Therefore after 15 seconds Casket will log:

.. code-block:: json

   {"level": "info", "msg": "returning Hello World"}

Example:

``CASKET_PYTHON_CODE_GATEWAY_TIMEOUT=15``
