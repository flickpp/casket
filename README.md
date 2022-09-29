# Casket

Casket is a Python WSGI gateway

## Building

Casket is written in Rust. You must obtain a copy of the Rust toolchain,
instructions for doing so are [here](https://www.rust-lang.org/tools/install).

Once obtained:

```
  # clone the source code
  $ git clone git@github.com:flickpp/casket.git
  # cd
  $ cd casket
  # Build Casket
  # cargo build --release
  # the casket binary is now target/release/casket
  $ cp target/release/tasket $INSTALL_DIR
```

## Running

To run a WSGI application, you must give casket the filename and the WSGI callable.

```python
# file service.py

from flask import Flask
app = Flask(__name__)

@app.route('/')
def hello_world():
    return 'Hello World!'
```

To run the above flask app:
```
   $ casket service:app
```

We note `service.py` is the filename and `app` is the name of the WSGI callable.

## Building Documentation

Documentation is done with sphinx.

To install sphinx with pip:

```
   $ pip install sphinx
```

And to build the docs:

```
   $ sphinx-build -b html doc build
```

Python has a HTTP server shipped in the standard library:

```
   $ cd build
   $ python -m http.server 9000
```

Docs can now be vied at http://localhost:9000
