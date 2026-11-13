MININET = mininet/*.py
TEST = mininet/test/*.py
EXAMPLES = mininet/examples/*.py
MN = bin/mn
PYTHON ?= python
PYMN = $(PYTHON) -B bin/mn
BIN = $(MN)
PYSRC = $(MININET) $(TEST) $(EXAMPLES) $(BIN)
MANPAGES = mn.1
P8IGN = E251,E201,E302,E202,E126,E127,E203,E226,E402,W504,W503,E731
PREFIX ?= /usr
BINDIR ?= $(PREFIX)/bin
MANDIR ?= $(PREFIX)/share/man/man1
DOCDIRS = doc/html doc/latex
PDF = doc/latex/refman.pdf

all: codecheck test

clean:
	rm -rf build dist *.egg-info *.pyc $(MANPAGES) $(DOCDIRS)

codecheck: $(PYSRC)
	-echo "Running code check"
	util/versioncheck.py
	pyflakes3 $(PYSRC) || pyflakes $(PYSRC)
	pylint --rcfile=.pylint $(PYSRC)
#	Exclude miniedit from pep8 checking for now
	pep8 --repeat --ignore=$(P8IGN) `ls $(PYSRC) | grep -v miniedit.py`

errcheck: $(PYSRC)
	-echo "Running check for errors only"
	pyflakes3 $(PYSRC) || pyflakes $(PYSRC)
	pylint -E --rcfile=.pylint $(PYSRC)

test: $(MININET) $(TEST)
	-echo "Running tests"
	mininet/test/test_nets.py
	mininet/test/test_hifi.py

slowtest: $(MININET)
	-echo "Running slower tests (walkthrough, examples)"
	mininet/test/test_walkthrough.py -v
	mininet/examples/test/runner.py -v

# NOTE: the C mnexec helper (mnexec.c) is removed from this tree; the
# mnexec binary the Python reference needs at runtime comes from the Rust
# port (apps/mnexec: cargo build --release -p mnexec) installed on PATH.

install-manpages: $(MANPAGES)
	install -D -t $(MANDIR) $(MANPAGES)

install: install-manpages
#	This seems to work on all pip versions
	$(PYTHON) -m pip uninstall -y mininet || true
	$(PYTHON) -m pip install .

develop: $(MANPAGES)
# 	Perhaps we should link these as well
	install $(MANPAGES) $(MANDIR)
	$(PYTHON) -m pip uninstall -y mininet || true
	$(PYTHON) -m pip install -e . --no-binary :all:

man: $(MANPAGES)

mn.1: $(MN)
	PYTHONPATH=. help2man -N -n "create a Mininet network." \
	--no-discard-stderr "$(PYMN)" -o $@

.PHONY: doc

doc: man
	doxygen doc/doxygen.cfg
	make -C doc/latex
