
Configuration
-------------------

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

``DEFAULT: 128``

The maximum number of open, but potentially idle TCP streams.
NOTE: This is global and NOT per worker.

If this limit is reached Casket will accept, then immediatly shutdown any
newer TCP streams.

Example:

``CASKET_MAX_CONNECTIONS=64``


CASKET_MAX_REQUESTS
~~~~~~~~~~~~~~~~~~~~~~~~

``DEFAULT: 12``

The maximum number of active HTTP requests, per worker.

If this limit is reached Casket will send back ``HTTP/1.1 503 Service Busy`` response header.

Example:

``CASKET_MAX_REQUESTS=8``


CASKET_RETURN_STACKTRACE_IN_BODY
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

``DEFAULT: 1``

When a Python exception is caught by Casket, optionally return the stacktrace as HTTP response body.

Casket will always return ``500 Internal Server Error`` if a Python exception is caught.
It does not matter what this value is set to.

Set this value to:

``CASKET_RETURN_STACKTRACE_IN_BODY=0`` (feature off)
``CASKET_RETURN_STACKTRACE_IN_BODY=1`` (feature on)
