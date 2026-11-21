"""Matplotlib backend for Corral: figures go to Mirador instead of a window.

Selected with MPLBACKEND=module://corral_mpl (corral_py sets it). Drawing is
Agg's; plt.show() and the end of every cell send the open figures to Mirador.
"""

from matplotlib.backend_bases import FigureManagerBase
from matplotlib.backends.backend_agg import FigureCanvasAgg


class FigureCanvasCorral(FigureCanvasAgg):
    manager_class = FigureManagerBase


FigureCanvas = FigureCanvasCorral
FigureManager = FigureManagerBase


def show(*args, **kwargs):
    import corral_py

    corral_py.flush_figures()
