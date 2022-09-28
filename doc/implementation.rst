.. _implementation:

Implementation
----------------

start_response
~~~~~~~~~~~~~~~~~

The `start_response <https://peps.python.org/pep-3333/#the-start-response-callable>`_
callable may be called according to the spec.
We use the code below to make some further points.

.. code-block:: python
   :linenos:

    def application(environ, start_response):
        status = "200 Ok"
	headers = [("HeaderName", "HeaderValue")]
        start_response(status, headers)
	return (b"",)

**Status**

The status string (line2) is parsed by casket to check it's valid.
We raise a RuntimeError if not.

We note that you can still use any reason string you like (e.g "200 Yes", "200 Boo") etc.

**Headers**

The headers (line3) are a list of (str, str) tuples.
Both the spec and Casket are insistent on these exact types.

If these exact types are not used, a TypeError is raised.

**Return Value**

start_response in Casket returns None.

This violates the spec, however the spec strongly encourages you from not using
the return value of start_response.

wsgi.input
~~~~~~~~~~~~~~~~~

Casket fully implements the ``wsgi.input`` protocol as specified
in the `input and errors streams <https://peps.python.org/pep-3333/#input-and-error-stream>`_.

We have further notes below this example.

.. code-block:: python
   :linenos:

   def application(environ, start_response):
       # Read 5 bytes or to end of stream - whichever happens first
       environ['wsgi.input'].read(5)

       start_response("200 Ok", [])
       return (b"",)


.. _environ-wsgi-errors:

wsgi.errors
~~~~~~~~~~~~~~

Casket fully implements the ``wsgi.errors`` protocol as specified
in the `input and errors streams <https://peps.python.org/pep-3333/#input-and-error-streams>`_.

We handle all sent error messages by immediatly logging to stdout.

.. code-block:: python
   :linenos:

   def application(environ, start_response):
       # produce line 1 in output below
       environ['wsgi.errors'].write("this is an error message")

       # produce lines 2 and 3 in output below
       environ['wsgi.errors'].writelines(["error message 2", "error message 3"])

       # We write to stdout immediatly so this is a no-op
       # It is avaliable as a function only for spec compliance
       environ['wsgi.errors'].flush()

       start_response("200 Ok", [])
       return (b'',)

       
Sending a request to execute the above code will produce the following on stdout.

.. code-block:: json
   :linenos:

   {"level":"error","ts":"2022-09-24T18:26:51.968364Z","msg":"application error","error":"this is an error message"}
   {"level":"error","ts":"2022-09-24T18:30:08.807870Z","msg":"application error","error":"error message 2"}
   {"level":"error","ts":"2022-09-24T18:30:08.807873Z","msg":"application error","error":"error message 3"}

   
**Method writelines**

The writelines method (line6) accepts any iterable of strings.


environ
~~~~~~~~~~~~

We populate the following standard variables in the environ dict

.. code-block:: python

   # REQUEST_METHOD as an uppercase string (e.g GET)
   environ['REQUEST_METHOD'] = "GET"

   # SCRIPT_NAME as the URL path (not including scheme, domain or query strings)
   environ['SCRIPT_NAME'] = "/user/12345/"

   # PATH_INFO is identical to SCRIPT_NAME
   environ['PATH_INFO'] = environ['SCRIPT_NAME']

   # QUERY_STRING is a percent encoded ascii string
   # This key may be omitted if the URL has no query string
   environ['QUERY_STRING'] = 'page=2'

   # CONTENT_TYPE is an arbitary string taken the from Content-Type header
   # This key may be omitted if the request does not use this header
   # No attempt to sanity check this Content-Type is recognised is done
   environ['CONTENT_TYPE'] = "application/json"

   # CONTENT_LENGTH is positive integer
   # It may be omitted if the request does not use it in the header
   environ['CONTENT_LENGTH'] = 64

   # SERVER_NAME is the hostname of the host
   # We read /etc/hostname on startup to get this value
   environ['SERVER_NAME'] = "myhostmachine"

   # SERVER_PORT is the port Casket is binded on
   # This is taken from the environment var CASKET_BIND_ADDR
   environ['SERVER_PORT'] = 8080

   # See above for these two values
   environ['wsgi.input'], envrion['wsgi.errors']

   # We then hardcode these three values
   environ['wsgi.multithread'] = True
   environ['wsgi.multiprocess'] = True;
   environ['wsgi.run_once'] = False


In addition Casket populates environ with the following extensions

**casket.trace_ctx**

This key is always present.

.. code-block:: python

   # A hexstring of exactly 64 chars
   environ['casket.trace_ctx'].trace_id

   # A hexstring of exactly 32 chars
   environ['casket.trace_ctx'].span_id

   # A hexstring of exactly 32 chars OR None
   environ['casket.trace_ctx'].parent_id
