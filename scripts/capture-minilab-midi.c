#define _POSIX_C_SOURCE 200809L

#include <alloca.h>
#include <alsa/asoundlib.h>
#include <errno.h>
#include <inttypes.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

typedef struct {
    int client;
    int port;
} Endpoint;

static volatile sig_atomic_t stopping = 0;
static pthread_mutex_t output_lock = PTHREAD_MUTEX_INITIALIZER;
static uint64_t output_order = 0;

static void stop_monitor(int signal_number) {
    (void)signal_number;
    stopping = 1;
}

static const char *event_name(unsigned char type) {
    switch (type) {
    case SND_SEQ_EVENT_NOTEOFF: return "note_off";
    case SND_SEQ_EVENT_NOTEON: return "note_on";
    case SND_SEQ_EVENT_KEYPRESS: return "poly_aftertouch";
    case SND_SEQ_EVENT_CONTROLLER: return "control_change";
    case SND_SEQ_EVENT_PGMCHANGE: return "program_change";
    case SND_SEQ_EVENT_CHANPRESS: return "channel_pressure";
    case SND_SEQ_EVENT_PITCHBEND: return "pitch_bend";
    case SND_SEQ_EVENT_SYSEX: return "sysex";
    case SND_SEQ_EVENT_QFRAME: return "mtc_quarter_frame";
    case SND_SEQ_EVENT_SONGPOS: return "song_position";
    case SND_SEQ_EVENT_SONGSEL: return "song_select";
    case SND_SEQ_EVENT_TUNE_REQUEST: return "tune_request";
    case SND_SEQ_EVENT_CLOCK: return "timing_clock";
    case SND_SEQ_EVENT_START: return "start";
    case SND_SEQ_EVENT_CONTINUE: return "continue";
    case SND_SEQ_EVENT_STOP: return "stop";
    case SND_SEQ_EVENT_SENSING: return "active_sensing";
    case SND_SEQ_EVENT_RESET: return "reset";
    default: return "other";
    }
}

static void print_quoted(const char *text) {
    putchar('"');
    for (const unsigned char *p = (const unsigned char *)text; *p != '\0'; ++p) {
        if (*p == '"' || *p == '\\') {
            putchar('\\');
            putchar(*p);
        } else if (*p >= 32 && *p < 127) {
            putchar(*p);
        } else {
            printf("\\x%02X", *p);
        }
    }
    putchar('"');
}

static void *read_labels(void *unused) {
    char label[512];
    (void)unused;
    while (!stopping && fgets(label, sizeof(label), stdin) != NULL) {
        struct timespec now;
        size_t length = strlen(label);
        while (length > 0 && (label[length - 1] == '\n' || label[length - 1] == '\r'))
            label[--length] = '\0';
        if (length == 0)
            continue;
        clock_gettime(CLOCK_MONOTONIC, &now);
        pthread_mutex_lock(&output_lock);
        printf("time=%lld.%09ld order=%" PRIu64
               " source=operator raw=NA status=NA type=marker label=",
               (long long)now.tv_sec, now.tv_nsec, ++output_order);
        print_quoted(label);
        putchar('\n');
        pthread_mutex_unlock(&output_lock);
    }
    return NULL;
}

static int source_is_allowed(const Endpoint *endpoints, int endpoint_count,
                             int client, int port) {
    for (int i = 0; i < endpoint_count; ++i) {
        if (endpoints[i].client == client && endpoints[i].port == port)
            return 1;
    }
    return 0;
}

static void source_names(snd_seq_t *seq, int client, int port,
                         char *client_name, size_t client_size,
                         char *port_name, size_t port_size) {
    snd_seq_client_info_t *client_info;
    snd_seq_port_info_t *port_info;
    snd_seq_client_info_alloca(&client_info);
    snd_seq_port_info_alloca(&port_info);
    snprintf(client_name, client_size, "?");
    snprintf(port_name, port_size, "?");
    if (snd_seq_get_any_client_info(seq, client, client_info) >= 0)
        snprintf(client_name, client_size, "%s", snd_seq_client_info_get_name(client_info));
    if (snd_seq_get_any_port_info(seq, client, port, port_info) >= 0)
        snprintf(port_name, port_size, "%s", snd_seq_port_info_get_name(port_info));
}

static void print_semantics(const snd_seq_event_t *event) {
    switch (event->type) {
    case SND_SEQ_EVENT_NOTEON:
        printf(" channel=%u note=%u velocity=%u release=%s",
               event->data.note.channel + 1, event->data.note.note,
               event->data.note.velocity, event->data.note.velocity == 0 ? "yes" : "no");
        break;
    case SND_SEQ_EVENT_NOTEOFF:
        printf(" channel=%u note=%u velocity=%u release=yes",
               event->data.note.channel + 1, event->data.note.note,
               event->data.note.velocity);
        break;
    case SND_SEQ_EVENT_KEYPRESS:
        printf(" channel=%u note=%u pressure=%u", event->data.note.channel + 1,
               event->data.note.note, event->data.note.velocity);
        break;
    case SND_SEQ_EVENT_CONTROLLER:
        printf(" channel=%u controller=%u value=%d", event->data.control.channel + 1,
               event->data.control.param, event->data.control.value);
        break;
    case SND_SEQ_EVENT_PGMCHANGE:
        printf(" channel=%u program=%d", event->data.control.channel + 1,
               event->data.control.value);
        break;
    case SND_SEQ_EVENT_CHANPRESS:
        printf(" channel=%u pressure=%d", event->data.control.channel + 1,
               event->data.control.value);
        break;
    case SND_SEQ_EVENT_PITCHBEND:
        printf(" channel=%u bend_signed=%d bend_14bit=%d", event->data.control.channel + 1,
               event->data.control.value, event->data.control.value + 8192);
        break;
    case SND_SEQ_EVENT_SYSEX:
        printf(" sysex_bytes=%u", event->data.ext.len);
        break;
    default:
        break;
    }
}

int main(int argc, char **argv) {
    snd_seq_t *seq = NULL;
    snd_midi_event_t *decoder = NULL;
    Endpoint *endpoints = NULL;
    pthread_t label_thread;
    int monitor_port;
    int endpoint_count;
    int exit_code = 0;

    if (argc < 2) {
        fprintf(stderr, "usage: %s CLIENT:PORT [CLIENT:PORT ...]\n", argv[0]);
        return 2;
    }
    endpoint_count = argc - 1;
    endpoints = calloc((size_t)endpoint_count, sizeof(*endpoints));
    if (endpoints == NULL) {
        fprintf(stderr, "cannot allocate endpoint list\n");
        return 1;
    }
    for (int i = 0; i < endpoint_count; ++i) {
        if (sscanf(argv[i + 1], "%d:%d", &endpoints[i].client,
                   &endpoints[i].port) != 2 || endpoints[i].client < 0 ||
            endpoints[i].port < 0) {
            fprintf(stderr, "invalid endpoint: %s\n", argv[i + 1]);
            free(endpoints);
            return 2;
        }
    }

    if (snd_seq_open(&seq, "default", SND_SEQ_OPEN_INPUT, 0) < 0) {
        fprintf(stderr, "cannot open ALSA sequencer\n");
        free(endpoints);
        return 1;
    }
    snd_seq_set_client_name(seq, "SHR MiniLab passive monitor");
    monitor_port = snd_seq_create_simple_port(
        seq, "receive only", SND_SEQ_PORT_CAP_WRITE | SND_SEQ_PORT_CAP_SUBS_WRITE,
        SND_SEQ_PORT_TYPE_MIDI_GENERIC | SND_SEQ_PORT_TYPE_APPLICATION);
    if (monitor_port < 0) {
        fprintf(stderr, "cannot create receive-only port\n");
        snd_seq_close(seq);
        free(endpoints);
        return 1;
    }
    for (int i = 0; i < endpoint_count; ++i) {
        int error = snd_seq_connect_from(seq, monitor_port, endpoints[i].client,
                                         endpoints[i].port);
        if (error < 0) {
            fprintf(stderr, "cannot subscribe from %s: %s\n", argv[i + 1],
                    snd_strerror(error));
            snd_seq_close(seq);
            free(endpoints);
            return 1;
        }
        fprintf(stderr, "subscribed source=%s destination=%d:%d\n", argv[i + 1],
                snd_seq_client_id(seq), monitor_port);
    }

    if (snd_midi_event_new(1024 * 1024, &decoder) < 0) {
        fprintf(stderr, "cannot create MIDI decoder\n");
        snd_seq_close(seq);
        free(endpoints);
        return 1;
    }
    snd_midi_event_no_status(decoder, 1);
    setvbuf(stdout, NULL, _IOLBF, 0);
    signal(SIGINT, stop_monitor);
    signal(SIGTERM, stop_monitor);
    if (pthread_create(&label_thread, NULL, read_labels, NULL) != 0) {
        fprintf(stderr, "cannot start label reader\n");
        snd_midi_event_free(decoder);
        snd_seq_close(seq);
        free(endpoints);
        return 1;
    }

    while (!stopping) {
        snd_seq_event_t *event = NULL;
        struct timespec now;
        unsigned char *raw;
        size_t raw_capacity = 32;
        long raw_length;
        char client_name[128];
        char port_name[128];
        int error = snd_seq_event_input(seq, &event);
        if (error < 0) {
            if (errno == EINTR || stopping)
                continue;
            fprintf(stderr, "ALSA event read failed: %s\n", snd_strerror(error));
            exit_code = 1;
            break;
        }
        if (!source_is_allowed(endpoints, endpoint_count, event->source.client,
                               event->source.port))
            continue;
        if (event->type == SND_SEQ_EVENT_SYSEX)
            raw_capacity = (size_t)event->data.ext.len + 1;
        raw = malloc(raw_capacity);
        if (raw == NULL) {
            fprintf(stderr, "cannot allocate raw MIDI buffer\n");
            exit_code = 1;
            break;
        }

        clock_gettime(CLOCK_MONOTONIC, &now);
        raw_length = snd_midi_event_decode(decoder, raw, raw_capacity, event);
        source_names(seq, event->source.client, event->source.port, client_name,
                     sizeof(client_name), port_name, sizeof(port_name));
        pthread_mutex_lock(&output_lock);
        printf("time=%lld.%09ld order=%" PRIu64 " source=%u:%u client=",
               (long long)now.tv_sec, now.tv_nsec, ++output_order,
               event->source.client, event->source.port);
        print_quoted(client_name);
        printf(" port=");
        print_quoted(port_name);
        printf(" raw=");
        if (raw_length > 0) {
            for (long i = 0; i < raw_length; ++i)
                printf("%s%02X", i == 0 ? "" : " ", raw[i]);
            printf(" status=0x%02X", raw[0]);
        } else {
            printf("NA status=NA");
        }
        printf(" type=%s alsa_type=%u", event_name(event->type), event->type);
        print_semantics(event);
        putchar('\n');
        pthread_mutex_unlock(&output_lock);
        free(raw);
    }

    stopping = 1;
    pthread_cancel(label_thread);
    pthread_join(label_thread, NULL);
    snd_midi_event_free(decoder);
    snd_seq_close(seq);
    free(endpoints);
    return exit_code;
}
