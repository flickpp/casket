Logger
----------------

.. code-block:: python

   from casket.logger import info, warn, error


This module contains four function (see above).
All functions have the same signature.

.. code-block:: python

   def info(msg, tags=None)


Tags may be omitted or may be a dictionary.
See :ref:`tut-logging` for examples.
