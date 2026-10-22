// netfp.c — read the §4.1 network fingerprint inputs WITHOUT root, the way
// serve would: per interface its flags (point-to-point), each address with its
// prefix length (and, for IPv6, temporary/deprecated), every default route
// with its interface, and the neighbour-table entry for each gateway.
//
// macOS: getifaddrs + SIOCGIFAFLAG_IN6 + sysctl(NET_RT_DUMP / NET_RT_FLAGS).
// Linux: getifaddrs + /proc/net/if_inet6 + rtnetlink (RTM_GETROUTE/GETNEIGH).
//
// Spike probe for docs/proposals/server-edition-phase2.md §4.1; not product
// code. Build: cc -O1 -Wall -o netfp netfp.c
#include <arpa/inet.h>
#include <errno.h>
#include <ifaddrs.h>
#include <net/if.h>
#include <netinet/in.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <unistd.h>

#ifdef __APPLE__
#include <net/if_dl.h>
#include <net/route.h>
#include <netinet6/in6_var.h>
#include <sys/sysctl.h>
#else
#include <linux/neighbour.h>
#include <linux/netlink.h>
#include <linux/rtnetlink.h>
#endif

static int prefixlen(const struct sockaddr *m) {
    if (!m) return -1;
    const uint8_t *b; size_t n;
    if (m->sa_family == AF_INET6) { b = ((const struct sockaddr_in6 *)m)->sin6_addr.s6_addr; n = 16; }
    else { b = (const uint8_t *)&((const struct sockaddr_in *)m)->sin_addr; n = 4; }
    int p = 0;
    for (size_t i = 0; i < n; i++) for (int k = 7; k >= 0; k--) if (b[i] >> k & 1) p++;
    return p;
}

static const char *ntop(const struct sockaddr *sa, char *buf, size_t len) {
    if (!sa) return "-";
    if (sa->sa_family == AF_INET) return inet_ntop(AF_INET, &((const struct sockaddr_in *)sa)->sin_addr, buf, len);
    if (sa->sa_family == AF_INET6) return inet_ntop(AF_INET6, &((const struct sockaddr_in6 *)sa)->sin6_addr, buf, len);
    return "?";
}

static void hex_ll(const uint8_t *p, int n, char *out) {
    out[0] = 0;
    if (n <= 0) { strcpy(out, "(none)"); return; }
    for (int i = 0; i < n; i++) sprintf(out + strlen(out), i ? ":%02x" : "%02x", p[i]);
}

#ifdef __APPLE__
static const char *v6flags(const char *ifname, const struct sockaddr_in6 *a) {
    static char out[128];
    out[0] = 0;
    int s = socket(AF_INET6, SOCK_DGRAM, 0);
    struct in6_ifreq r; memset(&r, 0, sizeof r);
    strlcpy(r.ifr_name, ifname, sizeof r.ifr_name);
    r.ifr_ifru.ifru_addr = *a;
    if (ioctl(s, SIOCGIFAFLAG_IN6, &r) < 0) { snprintf(out, sizeof out, "aflag-err=%d", errno); close(s); return out; }
    close(s);
    int f = r.ifr_ifru.ifru_flags6;
    if (f & IN6_IFF_TEMPORARY) strcat(out, "temporary,");
    if (f & IN6_IFF_DEPRECATED) strcat(out, "deprecated,");
    if (f & IN6_IFF_AUTOCONF) strcat(out, "autoconf,");
#ifdef IN6_IFF_SECURED
    if (f & IN6_IFF_SECURED) strcat(out, "secured,");
#endif
    if (f & IN6_IFF_TENTATIVE) strcat(out, "tentative,");
    if (f & IN6_IFF_DUPLICATED) strcat(out, "duplicated,");
    if (!out[0]) strcpy(out, "-");
    return out;
}
#else
static const char *v6flags(const char *ifname, const struct sockaddr_in6 *a) {
    static char out[128];
    strcpy(out, "not-in-if_inet6");
    FILE *f = fopen("/proc/net/if_inet6", "r");
    if (!f) return out;
    char hex[33], name[64]; unsigned idx, plen, scope, flags;
    while (fscanf(f, "%32s %x %x %x %x %63s", hex, &idx, &plen, &scope, &flags, name) == 6) {
        uint8_t b[16];
        for (int i = 0; i < 16; i++) { unsigned v; sscanf(hex + 2 * i, "%2x", &v); b[i] = v; }
        if (strcmp(name, ifname) || memcmp(b, a->sin6_addr.s6_addr, 16)) continue;
        out[0] = 0;
        if (flags & 0x01) strcat(out, "temporary,");     // IFA_F_TEMPORARY
        if (flags & 0x20) strcat(out, "deprecated,");    // IFA_F_DEPRECATED
        if (flags & 0x80) strcat(out, "permanent,");     // IFA_F_PERMANENT
        if (flags & 0x40) strcat(out, "tentative,");     // IFA_F_TENTATIVE
        if (flags & 0x100) strcat(out, "managetempaddr,");
        if (!out[0]) strcpy(out, "-");
        break;
    }
    fclose(f);
    return out;
}
#endif

static void interfaces(void) {
    struct ifaddrs *ifa, *i;
    if (getifaddrs(&ifa) < 0) { perror("getifaddrs"); return; }
    char a[64], d[64];
    for (i = ifa; i; i = i->ifa_next) {
        if (!i->ifa_addr) continue;
        int fam = i->ifa_addr->sa_family;
        if (fam != AF_INET && fam != AF_INET6) continue;
        if (!(i->ifa_flags & IFF_UP)) continue;
        printf("addr if=%s idx=%u ptp=%d loopback=%d broadcast=%d %s/%d",
               i->ifa_name, if_nametoindex(i->ifa_name),
               !!(i->ifa_flags & IFF_POINTOPOINT), !!(i->ifa_flags & IFF_LOOPBACK),
               !!(i->ifa_flags & IFF_BROADCAST),
               ntop(i->ifa_addr, a, sizeof a), prefixlen(i->ifa_netmask));
        if ((i->ifa_flags & IFF_POINTOPOINT) && i->ifa_dstaddr) printf(" dst=%s", ntop(i->ifa_dstaddr, d, sizeof d));
        if (fam == AF_INET6) {
            const struct sockaddr_in6 *s6 = (const void *)i->ifa_addr;
            printf(" scope_id=%u flags6=%s", s6->sin6_scope_id, v6flags(i->ifa_name, s6));
        }
        printf("\n");
    }
    freeifaddrs(ifa);
}

#ifdef __APPLE__
#define RT_ROUNDUP(a) ((a) > 0 ? (1 + (((a) - 1) | (sizeof(uint32_t) - 1))) : sizeof(uint32_t))

static char *rt_sysctl(int fam, int op, int arg, size_t *len, int *err) {
    int mib[6] = {CTL_NET, PF_ROUTE, 0, fam, op, arg};
    *err = 0;
    if (sysctl(mib, 6, NULL, len, NULL, 0) < 0) { *err = errno; return NULL; }
    char *buf = malloc(*len + 1);
    if (sysctl(mib, 6, buf, len, NULL, 0) < 0) { *err = errno; free(buf); return NULL; }
    return buf;
}

static void split_addrs(struct rt_msghdr *rtm, struct sockaddr **sa) {
    char *p = (char *)(rtm + 1);
    for (int i = 0; i < RTAX_MAX; i++) {
        sa[i] = NULL;
        if (!(rtm->rtm_addrs & (1 << i))) continue;
        sa[i] = (struct sockaddr *)p;
        p += RT_ROUNDUP(sa[i]->sa_len);
    }
}

// Every neighbour entry for `gw` (AF_INET: ARP, AF_INET6: NDP) as the
// sysctl hands it to this uid.
static void neigh_for(int fam, const struct sockaddr *gw, unsigned ifindex) {
    size_t len; int err;
    char *buf = rt_sysctl(fam, NET_RT_FLAGS, RTF_LLINFO, &len, &err);
    if (!buf) { printf("  neigh: sysctl NET_RT_FLAGS errno=%d (%s)\n", err, strerror(err)); return; }
    int seen = 0, total = 0;
    for (char *p = buf; p < buf + len;) {
        struct rt_msghdr *rtm = (struct rt_msghdr *)p;
        p += rtm->rtm_msglen;
        struct sockaddr *sa[RTAX_MAX];
        split_addrs(rtm, sa);
        total++;
        struct sockaddr *dst = sa[RTAX_DST];
        if (!dst || dst->sa_family != fam) continue;
        int same = fam == AF_INET
            ? ((struct sockaddr_in *)dst)->sin_addr.s_addr == ((const struct sockaddr_in *)gw)->sin_addr.s_addr
            : !memcmp(&((struct sockaddr_in6 *)dst)->sin6_addr, &((const struct sockaddr_in6 *)gw)->sin6_addr, 16);
        if (!same) continue;
        seen++;
        struct sockaddr_dl *sdl = (struct sockaddr_dl *)sa[RTAX_GATEWAY];
        char ll[64] = "(no gateway sa)";
        if (sdl && sdl->sdl_family == AF_LINK) hex_ll((uint8_t *)LLADDR(sdl), sdl->sdl_alen, ll);
        printf("  neigh: idx=%u lladdr=%s\n", rtm->rtm_index, ll);
    }
    printf("  neigh: %d entr%s for the gateway, %d entries in the whole NET_RT_FLAGS/RTF_LLINFO dump (ifindex wanted %u)\n",
           seen, seen == 1 ? "y" : "ies", total, ifindex);
    free(buf);
}

static void routes(int fam) {
    size_t len; int err;
    char *buf = rt_sysctl(fam, NET_RT_DUMP, 0, &len, &err);
    if (!buf) { printf("route: sysctl NET_RT_DUMP errno=%d\n", err); return; }
    char g[64], ifn[IF_NAMESIZE];
    for (char *p = buf; p < buf + len;) {
        struct rt_msghdr *rtm = (struct rt_msghdr *)p;
        p += rtm->rtm_msglen;
        struct sockaddr *sa[RTAX_MAX];
        split_addrs(rtm, sa);
        struct sockaddr *dst = sa[RTAX_DST], *mask = sa[RTAX_NETMASK], *gw = sa[RTAX_GATEWAY];
        if (!dst || dst->sa_family != fam) continue;
        int zero = fam == AF_INET ? ((struct sockaddr_in *)dst)->sin_addr.s_addr == 0
                                  : IN6_IS_ADDR_UNSPECIFIED(&((struct sockaddr_in6 *)dst)->sin6_addr);
        int masklen = (!mask || mask->sa_len == 0) ? 0 : prefixlen(mask);
        if (!zero || masklen != 0 || !(rtm->rtm_flags & RTF_GATEWAY)) continue;
        if (!if_indextoname(rtm->rtm_index, ifn)) strcpy(ifn, "?");
        const char *gws = gw && gw->sa_family == fam ? ntop(gw, g, sizeof g) : "(link)";
        printf("default fam=%s via=%s if=%s idx=%u ifscope=%d\n", fam == AF_INET ? "inet" : "inet6",
               gws, ifn, rtm->rtm_index, !!(rtm->rtm_flags & RTF_IFSCOPE));
        if (gw && gw->sa_family == fam) neigh_for(fam, gw, rtm->rtm_index);
    }
    free(buf);
}

int main(void) {
    printf("uid=%d\n", getuid());
    interfaces();
    routes(AF_INET);
    routes(AF_INET6);
    return 0;
}

#else  /* Linux */

static int nl_dump(int type, int fam, void (*cb)(struct nlmsghdr *)) {
    int s = socket(AF_NETLINK, SOCK_RAW | SOCK_CLOEXEC, NETLINK_ROUTE);
    if (s < 0) { perror("netlink socket"); return -1; }
    struct { struct nlmsghdr h; union { struct rtmsg r; struct ndmsg n; } u; } req;
    memset(&req, 0, sizeof req);
    req.h.nlmsg_len = sizeof req;
    req.h.nlmsg_type = type;
    req.h.nlmsg_flags = NLM_F_REQUEST | NLM_F_DUMP;
    if (type == RTM_GETROUTE) req.u.r.rtm_family = fam; else req.u.n.ndm_family = fam;
    if (send(s, &req, sizeof req, 0) < 0) { perror("netlink send"); close(s); return -1; }
    char buf[65536];
    for (;;) {
        ssize_t n = recv(s, buf, sizeof buf, 0);
        if (n <= 0) break;
        for (struct nlmsghdr *h = (void *)buf; NLMSG_OK(h, (size_t)n); h = NLMSG_NEXT(h, n)) {
            if (h->nlmsg_type == NLMSG_DONE) { close(s); return 0; }
            if (h->nlmsg_type == NLMSG_ERROR) { printf("netlink error\n"); close(s); return -1; }
            cb(h);
        }
    }
    close(s);
    return 0;
}

static struct { int fam; uint8_t gw[16]; int oif; unsigned table; } gws[32];
static int ngw;

static void route_cb(struct nlmsghdr *h) {
    struct rtmsg *r = NLMSG_DATA(h);
    if (r->rtm_dst_len != 0 || r->rtm_type != RTN_UNICAST) return;
    int len = RTM_PAYLOAD(h), oif = 0, has_gw = 0;
    unsigned table = r->rtm_table;
    uint8_t gw[16] = {0};
    for (struct rtattr *a = RTM_RTA(r); RTA_OK(a, len); a = RTA_NEXT(a, len)) {
        if (a->rta_type == RTA_OIF) oif = *(int *)RTA_DATA(a);
        if (a->rta_type == RTA_TABLE) table = *(unsigned *)RTA_DATA(a);
        if (a->rta_type == RTA_GATEWAY) { memcpy(gw, RTA_DATA(a), RTA_PAYLOAD(a)); has_gw = 1; }
    }
    char g[64] = "(link)", ifn[IF_NAMESIZE] = "?";
    if (has_gw) inet_ntop(r->rtm_family, gw, g, sizeof g);
    if_indextoname(oif, ifn);
    printf("default fam=%s via=%s if=%s idx=%d table=%u\n", r->rtm_family == AF_INET ? "inet" : "inet6", g, ifn, oif, table);
    if (has_gw && ngw < 32) { gws[ngw].fam = r->rtm_family; memcpy(gws[ngw].gw, gw, 16); gws[ngw].oif = oif; gws[ngw].table = table; ngw++; }
}

static int neigh_hits;
static void neigh_cb(struct nlmsghdr *h) {
    struct ndmsg *n = NLMSG_DATA(h);
    int len = h->nlmsg_len - NLMSG_LENGTH(sizeof *n);
    uint8_t dst[16] = {0}, ll[32]; int lln = 0;
    for (struct rtattr *a = (void *)((char *)n + NLMSG_ALIGN(sizeof *n)); RTA_OK(a, len); a = RTA_NEXT(a, len)) {
        if (a->rta_type == NDA_DST) memcpy(dst, RTA_DATA(a), RTA_PAYLOAD(a));
        if (a->rta_type == NDA_LLADDR) { lln = RTA_PAYLOAD(a); memcpy(ll, RTA_DATA(a), lln); }
    }
    for (int i = 0; i < ngw; i++) {
        if (gws[i].fam != n->ndm_family || memcmp(gws[i].gw, dst, n->ndm_family == AF_INET ? 4 : 16)) continue;
        char g[64], l[100];
        inet_ntop(n->ndm_family, dst, g, sizeof g);
        hex_ll(ll, lln, l);
        printf("  neigh: %s idx=%d lladdr=%s state=0x%x\n", g, n->ndm_ifindex, l, n->ndm_state);
        neigh_hits++;
    }
}

int main(void) {
    printf("uid=%d\n", getuid());
    interfaces();
    nl_dump(RTM_GETROUTE, AF_INET, route_cb);
    nl_dump(RTM_GETROUTE, AF_INET6, route_cb);
    nl_dump(RTM_GETNEIGH, AF_INET, neigh_cb);
    nl_dump(RTM_GETNEIGH, AF_INET6, neigh_cb);
    printf("  neigh: %d entr%s matched a gateway\n", neigh_hits, neigh_hits == 1 ? "y" : "ies");
    return 0;
}
#endif
