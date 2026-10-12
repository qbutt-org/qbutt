"""Generate small original EPUB, FB2 and TXT books for native reader QA."""

import base64
import shutil
import struct
import sys
import zipfile
import zlib
from pathlib import Path


def png() -> bytes:
    def chunk(kind: bytes, body: bytes) -> bytes:
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    pixels = b"".join(
        b"\0" + b"".join(bytes((0, 157, 247)) if x < 48 else bytes((255, 190, 80))
                          for x in range(96))
        for _ in range(64)
    )
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">2I5B", 96, 64, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b""))


def create_epub(path: Path, illustration: bytes) -> None:
    package = """<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="book-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="book-id">urn:uuid:11111111-2222-4333-8444-555555555555</dc:identifier>
    <dc:title>Native EPUB fixture</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="first" href="first.xhtml" media-type="application/xhtml+xml"/>
    <item id="second" href="second.xhtml" media-type="application/xhtml+xml"/>
    <item id="drawing" href="images/drawing.png" media-type="image/png"/>
  </manifest>
  <spine><itemref idref="first"/><itemref idref="second"/></spine>
</package>"""
    nav = """<html xmlns="http://www.w3.org/1999/xhtml"><body><nav epub:type="toc"
 xmlns:epub="http://www.idpf.org/2007/ops"><ol>
<li><a href="first.xhtml">Opening scene</a></li>
<li><a href="second.xhtml">Closing scene</a></li>
</ol></nav></body></html>"""
    first = """<html xmlns="http://www.w3.org/1999/xhtml"><body>
<h1>Opening scene</h1><p>Before <strong>bold</strong> and <em>italic</em> words.</p>
<ol><li>First task</li><li>Second task</li></ol>
<p><ruby>漢<rt>かん</rt></ruby> text follows.</p>
<figure><img src="images/drawing.png" alt="Blue and amber illustration"/></figure>
<p>After the illustration.</p>
</body></html>"""
    second = """<html xmlns="http://www.w3.org/1999/xhtml"><body>
<h1>Closing scene</h1><blockquote><p>A closing quotation.</p></blockquote>
<p>The end.</p></body></html>"""
    container = """<?xml version="1.0" encoding="utf-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="OPS/package.opf" media-type="application/oebps-package+xml"/>
</rootfiles></container>"""
    with zipfile.ZipFile(path, "w") as book:
        book.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        book.writestr("META-INF/container.xml", container)
        book.writestr("OPS/package.opf", package)
        book.writestr("OPS/nav.xhtml", nav)
        book.writestr("OPS/first.xhtml", first)
        book.writestr("OPS/second.xhtml", second)
        book.writestr("OPS/images/drawing.png", illustration)


def create_fragment_epub(path: Path, illustration: bytes) -> None:
    package = """<package xmlns="http://www.idpf.org/2007/opf" version="3.0"
 unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:identifier id="id">urn:uuid:66666666-7777-4888-9999-aaaaaaaaaaaa</dc:identifier>
<dc:title>Fragment navigation fixture</dc:title><dc:language>en</dc:language>
</metadata><manifest>
<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="body" href="body.xhtml" media-type="application/xhtml+xml"/>
<item id="drawing" href="images/drawing.png" media-type="image/png"/>
</manifest><spine><itemref idref="body"/></spine></package>"""
    nav = """<html xmlns="http://www.w3.org/1999/xhtml"><body><nav epub:type="toc"
 xmlns:epub="http://www.idpf.org/2007/ops"><ol>
<li><a href="body.xhtml#opening">Opening fragment</a></li>
<li><a href="body.xhtml#closing">Closing fragment</a></li>
</ol></nav></body></html>"""
    body = """<html xmlns="http://www.w3.org/1999/xhtml"><body>
<h1 id="opening">Opening fragment</h1><p>The first section.</p>
<svg xmlns="http://www.w3.org/2000/svg"><image href="images/drawing.png"/></svg>
<h1 id="closing">Closing fragment</h1><p>The second section.</p>
""" + "".join(f"<p>Closing paragraph {number}: The chapter continues for a position check.</p>"
             for number in range(1, 26)) + "</body></html>"
    container = """<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"
 version="1.0"><rootfiles><rootfile full-path="OPS/package.opf"
 media-type="application/oebps-package+xml"/></rootfiles></container>"""
    with zipfile.ZipFile(path, "w") as book:
        book.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        book.writestr("META-INF/container.xml", container)
        book.writestr("OPS/package.opf", package)
        book.writestr("OPS/nav.xhtml", nav)
        book.writestr("OPS/body.xhtml", body)
        book.writestr("OPS/images/drawing.png", illustration)


def create_fb2(path: Path, illustration: bytes) -> None:
    encoded = base64.b64encode(illustration).decode("ascii")
    book = f"""<?xml version="1.0" encoding="utf-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0"
 xmlns:l="http://www.w3.org/1999/xlink">
<description><title-info><genre>prose</genre><author><first-name>Fixture</first-name>
<last-name>Author</last-name></author><book-title>Native FB2 fixture</book-title>
<lang>en</lang></title-info></description>
<body><section><title><p>First section</p></title>
<p>Text with <strong>bold</strong> and <emphasis>italic</emphasis> words.</p>
<image l:href="#drawing"/><p>After the illustration.</p>
<section><title><p>Nested section</p></title><p>Nested text.</p></section></section>
<section><title><p>Second section</p></title><p>The end.</p></section></body>
<binary id="drawing" content-type="image/png">{encoded}</binary>
</FictionBook>"""
    path.write_text(book, encoding="utf-8")


def main() -> None:
    destination = Path(sys.argv[1])
    destination.mkdir(parents=True, exist_ok=True)
    illustration = png()
    create_epub(destination / "native.epub", illustration)
    create_fragment_epub(destination / "fragments.epub", illustration)
    shutil.copyfile(destination / "native.epub", destination / "unsafe.epub")
    with zipfile.ZipFile(destination / "unsafe.epub", "a") as book:
        book.writestr("../escape", "must never be extracted")
    create_fb2(destination / "native.fb2", illustration)
    (destination / "native.txt").write_text("Opening line.\nAnother line.\n\nClosing paragraph.\n", encoding="utf-8")
    (destination / "bad-encoding.fb2").write_bytes(b'<?xml version="1.0" encoding=\xc3\xa9?>')


if __name__ == "__main__":
    main()
