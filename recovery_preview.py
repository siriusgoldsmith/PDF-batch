"""Read-only visual references to controller-sent pages, not proof of printing."""


def _positive_page(value):
    return value if type(value) is int and value > 0 else 0


def recovery_reference(items, item_id):
    """Return selected progress or the nearest earlier sent document's progress.

    Queue order is (position, id). Only positive integer journal pages count;
    a positive total_pages caps them. Invalid progress is treated as no progress.
    Returned item data is a copy; no journal state is changed.
    """
    previous = None
    for source in sorted(items, key=lambda item: (item['position'], item['id'])):
        item = dict(source)
        page = _positive_page(item.get('last_page'))
        total = _positive_page(item.get('total_pages'))
        if total:
            page = min(page, total)
        reference = {'item': item, 'page': page} if page else None
        if item['id'] == item_id:
            return reference or previous
        if item['state'] == 'sent' and reference:
            previous = reference
    return None


def render_reference(reference, max_size=(440, 500)):
    """Return an in-memory RGB Pillow image, respecting PDF rotation and bounds.

    Reference pages are 1-based. Invalid references, pages or pixel bounds raise
    ValueError; snapshot access/decoding errors propagate from PyMuPDF. The PDF
    is closed before returning, and no preview file is written.
    """
    import fitz
    from PIL import Image

    if (not isinstance(max_size, (tuple, list)) or len(max_size) != 2
            or not all(_positive_page(value) for value in max_size)):
        raise ValueError('max_size must contain two positive integer pixel sizes')
    try:
        page_number = _positive_page(reference['page'])
        snapshot = reference['item']['snapshot']
    except (KeyError, TypeError) as exc:
        raise ValueError('Reference must contain an item snapshot and page') from exc
    if not page_number:
        raise ValueError('Reference page must be a positive integer')
    with fitz.open(snapshot) as document:
        if page_number > document.page_count:
            raise ValueError('Reference page is outside the snapshot')
        page = document[page_number - 1]
        scale = min(max_size[0] / page.rect.width, max_size[1] / page.rect.height)
        pixmap = page.get_pixmap(matrix=fitz.Matrix(scale, scale),
                                 colorspace=fitz.csRGB, alpha=False)
        return Image.frombytes('RGB', (pixmap.width, pixmap.height), pixmap.samples)
