
Tutorial - Casket with Flask and Docker
-------------------------------------------


Creating a Virtual Environment and Installing Flask
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

First we create a virtual environment to work in and install Flask.

.. code-block:: bash

   $ python -m venv venv
   $ source venv/bin/activate
   $ pip install Flask


Next we have to implement our application.
It has one endpoint '/' which is accessed with the GET method.

.. code-block:: python

   from flask import Flask

   app = Flask("casket-example")

   @app.route('/')
   def hello_world():
       return 'Hello World!'


Writing the Dockerfile
~~~~~~~~~~~~~~~~~~~~~~~~~~

First we must pull in any dependencies (including flask).
To do this first *freeze* your environment.

.. code-block:: bash

   pip freeze -l > requirements.txt


Now we're ready to write our Dockerfile

.. code-block:: Dockerfile

    FROM flickpp/casket:latest
    
    # Install dependencies
    COPY requirements.txt requirements.txt
    RUN pip install -r requirements.txt

    # Install our application
    COPY app.py app.py

    # Run casket
    CMD ["casket", "app:app"]

    
Building and Running
~~~~~~~~~~~~~~~~~~~~~~~~~

We must first build the docker image - then run a container.

.. code-block:: shell

   $ docker build . -t casket-example:latest
   $ docker run casket-example:latest

