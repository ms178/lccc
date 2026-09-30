/* Oracle reproducer (tests/oracle/delta_corpus.json): Expat xmltok UTF-8 name scan + document hash walk
 * Extracted verbatim from tests/benchmark/programs/expat_xml_scan.c; the measured
 * functions are exported so no compiler can inline them away.  Provenance:
 * tests/benchmark/WORKLOAD_PROVENANCE.md. */

static int
xml_name_start(unsigned char c)
{
  return (c >= (unsigned char)'a' && c <= (unsigned char)'z')
      || (c >= (unsigned char)'A' && c <= (unsigned char)'Z')
      || c == (unsigned char)'_' || c == (unsigned char)':'
      || c >= 0xc2U;
}

static int
xml_name_continue(unsigned char c)
{
  return xml_name_start(c)
      || (c >= (unsigned char)'0' && c <= (unsigned char)'9')
      || c == (unsigned char)'-' || c == (unsigned char)'.';
}

/* UTF-8 specialization of Expat's BYTE_TYPE/LEAD_CASE progression. */
unsigned long
expat_utf8_name_length(const unsigned char *ptr, const unsigned char *end)
{
  const unsigned char *start = ptr;

  while (ptr < end) {
    unsigned char c = *ptr;
    if (c < 0x80U) {
      if (!xml_name_continue(c))
        break;
      ptr++;
    } else {
      unsigned long width;
      if (c >= 0xc2U && c <= 0xdfU)
        width = 2UL;
      else if (c >= 0xe0U && c <= 0xefU)
        width = 3UL;
      else if (c >= 0xf0U && c <= 0xf4U)
        width = 4UL;
      else
        break;
      if ((unsigned long)(end - ptr) < width)
        break;
      if ((ptr[1] & 0xc0U) != 0x80U
          || (width > 2UL && (ptr[2] & 0xc0U) != 0x80U)
          || (width > 3UL && (ptr[3] & 0xc0U) != 0x80U))
        break;
      ptr += width;
    }
  }
  return (unsigned long)(ptr - start);
}

unsigned long
expat_scan_document(const unsigned char *ptr, const unsigned char *end)
{
  unsigned long hash = 1469598103934665603UL;

  while (ptr < end) {
    unsigned char c = *ptr;
    if (c == (unsigned char)'"' || c == (unsigned char)'\'') {
      unsigned char quote = c;
      ptr++;
      while (ptr < end && *ptr != quote)
        ptr++;
      if (ptr < end)
        ptr++;
    } else if (xml_name_start(c)) {
      unsigned long len = expat_utf8_name_length(ptr, end);
      /* The caller only invokes the kernel for valid starts. */
      if (len == 0UL)
        return hash;
      hash ^= len + (unsigned long)c;
      hash *= 1099511628211UL;
      ptr += len;
    } else {
      ptr++;
    }
  }
  return hash;
}
