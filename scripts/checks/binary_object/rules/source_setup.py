"""Source setup checks, in the ordered boundary scan."""
from __future__ import annotations

import re


def check(ctx):
    ctx.raw_string_prefix = re.compile(r'(?:br|rb|cr|rc|r)(?P<hashes>#{0,255})"')
