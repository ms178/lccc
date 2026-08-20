#include <stdio.h>
#include <string.h>
#include <expat.h>
static unsigned long long H = 1469598103934665603ULL;
static void mix(unsigned long long v){ H=(H^(v*2654435761ULL))*1099511628211ULL; }
static void ms(const char*s){ unsigned long long h=5381; for(;*s;s++)h=h*33+(unsigned char)*s; mix(h); }
static void XMLCALL start(void *ud, const XML_Char *n, const XML_Char **at){
    (void)ud; ms("S"); ms(n);
    for (int i=0; at[i]; i+=2){ ms(at[i]); mix(strtoll(at[i+1],0,10)); ms(at[i+1]); }
}
static void XMLCALL end(void *ud, const XML_Char *n){ (void)ud; ms("E"); ms(n); }
static void XMLCALL chard(void *ud, const XML_Char *s, int len){ (void)ud; unsigned long long h=5381; for(int i=0;i<len;i++)h=h*33+(unsigned char)s[i]; mix(h); }
int main(void){
    const char *docs[] = {
      "<?xml version='1.0'?><root a='1' b='2'><x k='3'>hello</x><y/><z w='4'>tail</z></root>",
      "<!DOCTYPE r [<!ELEMENT r ANY><!ENTITY e 'expanded'>]><r>&e; and more</r>",
      "<a><b><c><d>deep</d></c></b></a>",
      "<nums><n>1</n><n>22</n><n>333</n></nums>",
      "<bad>&undefined;</bad>",
      "<unclosed>",
      0 };
    for (int i=0; docs[i]; i++){
        XML_Parser p = XML_ParserCreate(NULL);
        XML_SetElementHandler(p, start, end);
        XML_SetCharacterDataHandler(p, chard);
        int rc = XML_Parse(p, docs[i], (int)strlen(docs[i]), 1);
        mix(rc); mix(XML_GetErrorCode(p));
        XML_ParserFree(p);
    }
    printf("%llx\n", H);
    return 0;
}
