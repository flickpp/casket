JSON logging and Trace Ids
------------------------------

Casket supports ndjson logging (newline delimited json).
Here is a quickstart example.

.. code-block:: python

   from casket import logger

   def application(environ, start_response):
       logger.info("hello world")
       start_response("200 Ok", [])
       return (b"",)

       
Which will produce an output similar to the following:

.. code-block:: json

   {"level":"info","ts":"2022-09-30T17:03:59.212990Z","msg":"hello wrld","trace_id":"08f96edbf143fd5f0c5b52d649c2ffa3","span_id":"7fd4ab27e372da2c"}


Trace Ids
~~~~~~~~~~~~~~

If we execute the above code inside Casket and send a request we get the following response headers:

.. code-block::

   < HTTP/1.1 200 Ok
   < X-TraceId: 08f96edbf143fd5f0c5b52d649c2ffa3
   < Connection: Keep-Alive
   < Server: Casket


We are particularly interested in the X-TraceId header.

* All requests sent to Casket get **exactly one** trace id
* All log lines while executing the request will log the trace id
* Casket will **always** return X-TraceId header.


Logging Tags
~~~~~~~~~~~~~~~~~

Casket logger functions support adding additional logging tags.
Consider this example.

.. code-block:: python

   def application(environ, start_response):
       logger.info("an example with tags", {
           "key1": "we can log additional k/v pairs",
	   "key2": True,
	   "key3": 98.1,
       })
   
       start_response("200 Ok", [])
       return (b"",)


Running this example code will produce a log line containing the additional three keys.
We also note that the JSON serializer supports True/False and numbers from Python.

Piping (tips)
~~~~~~~~~~~~~~~

Casket *always* logs to stdout, with one log message per line.
This makes processing the stream simple with pipes.

The below example will only print log lines containing an error to your terminal.

.. code-block::

   $ casket service:app | grep error


Logging to a file is likewise trivial.

.. code-block::

   $ casket service:app > logfile


Searching this file is also simple - a common thing to do is grep for a trace_id.

.. code-block::

   $ grep 08f96edbf143fd5f0c5b52d649c2ffa3 logfile
