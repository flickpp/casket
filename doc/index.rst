###########
Casket
###########

Cakset is a Python WSGI gateway.

| Status: Alpha
| Avaliability: UNIX

Quickstart
===================

.. code-block:: python

   # File service.py

   from flask import Flask
   app = Flask(__name__)

   @app.route('/')
   def hello_world():
       return 'Hello World!'

    
Then

.. code-block:: shell

   $ casket service:app


Obtaining Casket
===================

Casket is avaliable as source code.
We do not currently release binaries.

Source code with instructions for building are avaliable in the `repo <https://github.com/flickpp/casket>`_.


Tutorials
=============

.. toctree::
   tut-logging.rst


Modules
===========

.. toctree::
   logger.rst

Releases
============

.. toctree::
   releases.rst


Reference
==================

.. toctree::
   statuscodes.rst
   configuration.rst
   implementation.rst
