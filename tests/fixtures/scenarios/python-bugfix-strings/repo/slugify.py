"""Pinned evaluation fixture for issue #136 Slice A. Intentionally broken:
``slugify`` does not collapse whitespace runs to single dashes. Do not "fix"
this file in place - the content digest is pinned by the scenario descriptor.
"""


def slugify(text: str) -> str:
    """Lowercase ``text``, strip the edges and join words with single dashes."""
    return text.strip().lower()


if __name__ == "__main__":
    assert slugify("Hello   World") == "hello-world", "fixture is intentionally broken"
