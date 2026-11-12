# Sphinx configuration for the Mininet documentation (Python + Rust port).
# Build:  pip install -r docs/requirements.txt && make -C docs html

import os
import sys

sys.path.insert(0, os.path.abspath("."))

project = "Mininet"
copyright = "2011-2026, The Mininet Contributors"
author = "Bob Lantz and the Mininet Contributors"
version = "2.3.1b4"
release = "2.3.1b4 + rust-migration"

extensions = [
    "sphinx.ext.autodoc",
    "sphinx.ext.viewcode",
    "sphinx.ext.intersphinx",
    "myst_parser",
]

myst_enable_extensions = ["colon_fence", "deflist"]

templates_path = ["_templates"]
exclude_patterns = ["_build", "Thumbs.db", ".DS_Store", "README.md"]

html_theme = "furo"
html_title = "Mininet — network emulator in Python & Rust"
html_theme_options = {
    "sidebar_hide_name": False,
    "navigation_with_keys": True,
    "top_of_page_buttons": ["view", "edit"],
}
html_static_path = ["_static"]
html_css_files = ["custom.css"]

intersphinx_mapping = {
    "python": ("https://docs.python.org/3", None),
}

# Treat warnings as errors in CI-style builds:  make -C docs html SPHINXOPTS="-W"
nitpicky = False
