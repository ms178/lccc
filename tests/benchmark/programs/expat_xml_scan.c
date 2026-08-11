/*
 * Workload-derived kernel: Expat 2.8.2, lib/xmltok_impl.c
 * (`nameLength` and UTF-8 token classification behavior).
 *
 * SPDX-License-Identifier: MIT
 *
 * Expat's tokenizer is parameterized over multiple encodings and project
 * macros.  This standalone workload specializes that hot name-scanning path
 * to validated UTF-8 input, retaining its ASCII fast path, multi-byte checks,
 * and end-pointer discipline.  It is not an XML parser or a replacement for
 * Expat.  See tests/benchmark/WORKLOAD_PROVENANCE.md for exact source and
 * adaptation boundaries.
 *
 * The generated corpus contains tags, attributes, quotes, entities, and UTF-8
 * names.  It stresses branch prediction, byte classification, bounds checks,
 * and pointer induction in a parser-shaped loop.
 */
#include <stdio.h>

#define XML_SIZE (1UL << 20)
#define PASSES 64U

static unsigned char expat_xml_data[XML_SIZE];

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
static unsigned long
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

static unsigned long
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

static void
make_xml_corpus(void)
{
  static const unsigned char fragment[] =
      "<caf\303\251 data-id=\"17\" role=\"entry_42\">"
      "text &amp; more</caf\303\251>\n";
  unsigned long pos = 0UL;
  unsigned long j;

  while (pos + sizeof(fragment) <= XML_SIZE) {
    for (j = 0UL; j + 1UL < sizeof(fragment); j++)
      expat_xml_data[pos++] = fragment[j];
  }
  while (pos < XML_SIZE)
    expat_xml_data[pos++] = (unsigned char)' ';
}

static int
check_name_kernel(void)
{
  static const unsigned char ascii_name[] = "alpha-9";
  static const unsigned char utf8_name[] = "caf\303\251Tag";

  if (expat_utf8_name_length(ascii_name, ascii_name + 7) != 7UL)
    return 0;
  if (expat_utf8_name_length(utf8_name, utf8_name + 8) != 8UL)
    return 0;
  return 1;
}

int
main(void)
{
  unsigned int pass;
  unsigned long checksum = 0UL;

  if (!check_name_kernel())
    return 2;
  make_xml_corpus();

  for (pass = 0; pass < PASSES; pass++) {
    checksum ^= expat_scan_document(expat_xml_data,
                                    expat_xml_data + XML_SIZE);
    /* A bounded byte mutation gives every pass a distinct data path. */
    expat_xml_data[(pass * 8191U) & (XML_SIZE - 1UL)] ^= 1U;
  }

  printf("%lu\n", checksum);
  return 0;
}
