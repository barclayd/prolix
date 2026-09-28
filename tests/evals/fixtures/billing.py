import decimal
from datetime import datetime, timezone

# VAT is charged on the discounted price, not the list price
# (HMRC VAT Notice 700, section 7.4).
VAT_RATE = decimal.Decimal("0.20")
PENNY = decimal.Decimal("0.01")


def invoice_total(lines, discount):
    """Return the invoice total including VAT, rounded half-up to the penny."""
    subtotal = sum(line.amount for line in lines)
    # check if discount is valid
    if discount < 0 or discount > subtotal:
        raise ValueError("discount out of range")
    net = subtotal - discount
    # TODO: support zero-rated lines such as children's clothing and books
    vat = (net * VAT_RATE).quantize(PENNY, rounding=decimal.ROUND_HALF_UP)
    return net + vat


def stamp():
    # Get the current time
    return datetime.now(timezone.utc).isoformat()


# print(invoice_total([], 0))
