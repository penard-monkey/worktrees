// mdns_reg_mac.c — register a TEST _worktrees._tcp advert through
// mDNSResponder (DNSServiceRegister) on ONE interface index, hold it, then
// withdraw it: `release` deallocates the ref (serve's clean withdrawal),
// `kill` SIGKILLs this process (a crashed serve).
//
// Usage: mdns_reg_mac <ifname> <port> <hold-seconds> release|kill [instance]
// The instance name and the TXT `id` are a random 26-char base32 id, as the
// §2.1 discovery id would be. Nothing listens on <port>.
// Build: cc -O1 -Wall -o mdns_reg_mac mdns_reg_mac.c   (dns_sd is in libSystem)
// Spike probe for docs/proposals/server-edition-phase2.md §4.7; not product code.
#include <dns_sd.h>
#include <arpa/inet.h>
#include <net/if.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/select.h>
#include <sys/time.h>
#include <unistd.h>

static double now(void) { struct timeval t; gettimeofday(&t, NULL); return t.tv_sec + t.tv_usec / 1e6; }

static void cb(DNSServiceRef r, DNSServiceFlags f, DNSServiceErrorType e, const char *name,
               const char *type, const char *domain, void *ctx) {
    (void)r; (void)ctx;
    printf("[reg %.3f] callback err=%d flags=0x%x name=%s type=%s domain=%s\n", now(), e, f, name, type, domain);
    fflush(stdout);
}

int main(int argc, char **argv) {
    if (argc < 5) { fprintf(stderr, "usage: %s ifname port hold release|kill [instance]\n", argv[0]); return 2; }
    uint32_t idx = if_nametoindex(argv[1]);
    uint16_t port = (uint16_t)atoi(argv[2]);
    double hold = atof(argv[3]);
    char id[27];
    if (argc > 5) snprintf(id, sizeof id, "%s", argv[5]);
    else {
        const char *a = "abcdefghijklmnopqrstuvwxyz234567";
        arc4random_buf(id, 26);
        for (int i = 0; i < 26; i++) id[i] = a[(unsigned char)id[i] % 32];
        id[26] = 0;
    }
    TXTRecordRef txt;
    TXTRecordCreate(&txt, 0, NULL);
    TXTRecordSetValue(&txt, "v", 1, "1");
    TXTRecordSetValue(&txt, "id", 26, id);
    DNSServiceRef ref;
    double t0 = now();
    DNSServiceErrorType e = DNSServiceRegister(&ref, kDNSServiceFlagsNoAutoRename, idx, id, "_worktrees._tcp",
                                               NULL, NULL, htons(port), TXTRecordGetLength(&txt),
                                               TXTRecordGetBytesPtr(&txt), cb, NULL);
    printf("[reg %.3f] DNSServiceRegister(if=%s idx=%u, name=%s, port=%u) -> %d\n", t0, argv[1], idx, id, port, e);
    fflush(stdout);
    if (e) return 1;
    int fd = DNSServiceRefSockFD(ref);
    double end = now() + hold;
    while (now() < end) {
        fd_set s; FD_ZERO(&s); FD_SET(fd, &s);
        struct timeval tv = {0, 200000};
        if (select(fd + 1, &s, NULL, NULL, &tv) > 0) DNSServiceProcessResult(ref);
    }
    if (!strcmp(argv[4], "kill")) {
        printf("[reg %.3f] SIGKILL self\n", now()); fflush(stdout);
        kill(getpid(), SIGKILL);
    }
    DNSServiceRefDeallocate(ref);
    printf("[reg %.3f] DNSServiceRefDeallocate — withdrawn\n", now()); fflush(stdout);
    TXTRecordDeallocate(&txt);
    return 0;
}
