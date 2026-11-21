# Run by `corral` when it opens the Python console: ipython -i py/startup.py
# It switches Corral on and leaves nothing behind in your namespace.


def _corral_start():
    import os
    import sys

    here = os.path.dirname(os.path.abspath(__file__))
    if here not in sys.path:
        sys.path.insert(0, here)
    import corral_py

    corral_py.activate()


_corral_start()
del _corral_start
