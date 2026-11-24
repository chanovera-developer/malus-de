/*
 * Malus DE libunity.so.9 D-Bus LauncherEntry bridge
 *
 * Implements the minimal UnityLauncherEntry ABI loaded via dlopen("libunity.so.9")
 * by Chromium-based browsers (Brave, Brave Origin, Chrome, Chromium, Edge, Vivaldi,
 * Opera) and Electron apps, forwarding download progress and badges directly over
 * the standard `com.canonical.Unity.LauncherEntry.Update` Session D-Bus signal.
 */

#include <gio/gio.h>
#include <glib-object.h>
#include <math.h>
#include <stdint.h>
#include <string.h>

typedef struct _MalusUnityLauncherEntry MalusUnityLauncherEntry;
typedef struct _MalusUnityLauncherEntryClass MalusUnityLauncherEntryClass;

struct _MalusUnityLauncherEntry {
    GObject parent_instance;
    GMutex lock;
    gchar *app_uri;
    gchar *object_path;
    gint64 count;
    gboolean count_visible;
    gdouble progress;
    gboolean progress_visible;
    gboolean urgent;
    gdouble last_emitted_progress;
    gint64 last_emit_us;
};

struct _MalusUnityLauncherEntryClass {
    GObjectClass parent_class;
};

GType malus_unity_launcher_entry_get_type(void);
G_DEFINE_TYPE(MalusUnityLauncherEntry, malus_unity_launcher_entry, G_TYPE_OBJECT)

typedef struct _MalusUnityInspector MalusUnityInspector;
typedef struct _MalusUnityInspectorClass MalusUnityInspectorClass;

struct _MalusUnityInspector {
    GObject parent_instance;
};

struct _MalusUnityInspectorClass {
    GObjectClass parent_class;
};

enum {
    PROP_INSPECTOR_0,
    PROP_UNITY_RUNNING,
};

GType malus_unity_inspector_get_type(void);
G_DEFINE_TYPE(MalusUnityInspector, malus_unity_inspector, G_TYPE_OBJECT)

static void malus_unity_inspector_get_property(
    GObject *object, guint prop_id, GValue *value, GParamSpec *pspec) {
    (void)object;
    switch (prop_id) {
        case PROP_UNITY_RUNNING:
            g_value_set_boolean(value, TRUE);
            break;
        default:
            G_OBJECT_WARN_INVALID_PROPERTY_ID(object, prop_id, pspec);
            break;
    }
}

static void malus_unity_inspector_class_init(MalusUnityInspectorClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->get_property = malus_unity_inspector_get_property;
    g_object_class_install_property(
        object_class,
        PROP_UNITY_RUNNING,
        g_param_spec_boolean(
            "unity-running",
            "Unity Running",
            "Whether the Malus DE LauncherEntry bridge is active",
            TRUE,
            G_PARAM_READABLE | G_PARAM_STATIC_STRINGS));
}

static void malus_unity_inspector_init(MalusUnityInspector *self) {
    (void)self;
}

static GMutex g_registry_lock;
static GHashTable *g_entries = NULL;
static GDBusConnection *g_session_bus = NULL;
static MalusUnityInspector *g_inspector = NULL;

__attribute__((visibility("default")))
gpointer unity_inspector_get_default(void) {
    g_mutex_lock(&g_registry_lock);
    if (g_inspector == NULL) {
        g_inspector =
            (MalusUnityInspector *)g_object_new(malus_unity_inspector_get_type(), NULL);
    }
    gpointer ret = g_inspector;
    g_mutex_unlock(&g_registry_lock);
    return ret;
}

__attribute__((visibility("default")))
gboolean unity_inspector_get_unity_running(gpointer inspector_ptr) {
    (void)inspector_ptr;
    return TRUE;
}

static void malus_unity_launcher_entry_finalize(GObject *obj) {
    MalusUnityLauncherEntry *self = (MalusUnityLauncherEntry *)obj;
    g_free(self->app_uri);
    g_free(self->object_path);
    g_mutex_clear(&self->lock);
    G_OBJECT_CLASS(malus_unity_launcher_entry_parent_class)->finalize(obj);
}

static void malus_unity_launcher_entry_class_init(MalusUnityLauncherEntryClass *klass) {
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->finalize = malus_unity_launcher_entry_finalize;
}

static void malus_unity_launcher_entry_init(MalusUnityLauncherEntry *self) {
    g_mutex_init(&self->lock);
    self->last_emitted_progress = -1.0;
}

static GDBusConnection *get_session_bus_locked(void) {
    if (g_session_bus != NULL && !g_dbus_connection_is_closed(g_session_bus)) {
        return g_session_bus;
    }
    g_clear_object(&g_session_bus);
    g_session_bus = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, NULL);
    return g_session_bus;
}

static void emit_update_locked(MalusUnityLauncherEntry *entry) {
    if (entry == NULL || entry->app_uri == NULL || entry->object_path == NULL) {
        return;
    }

    g_mutex_lock(&g_registry_lock);
    GDBusConnection *bus = get_session_bus_locked();
    if (bus != NULL) {
        g_object_ref(bus);
    }
    g_mutex_unlock(&g_registry_lock);

    if (bus == NULL) {
        return;
    }

    GVariantBuilder dict;
    g_variant_builder_init(&dict, G_VARIANT_TYPE("a{sv}"));
    g_variant_builder_add(&dict, "{sv}", "count", g_variant_new_int64(entry->count));
    g_variant_builder_add(&dict, "{sv}", "count-visible", g_variant_new_boolean(entry->count_visible));
    g_variant_builder_add(&dict, "{sv}", "progress", g_variant_new_double(entry->progress));
    g_variant_builder_add(
        &dict, "{sv}", "progress-visible", g_variant_new_boolean(entry->progress_visible));
    g_variant_builder_add(&dict, "{sv}", "urgent", g_variant_new_boolean(entry->urgent));

    GVariant *params = g_variant_new("(sa{sv})", entry->app_uri, &dict);
    g_dbus_connection_emit_signal(
        bus,
        NULL,
        entry->object_path,
        "com.canonical.Unity.LauncherEntry",
        "Update",
        params,
        NULL);
    g_object_unref(bus);
}

static gchar *normalize_app_uri(const gchar *desktop_id) {
    if (desktop_id == NULL || desktop_id[0] == '\0') {
        return g_strdup("application://unknown.desktop");
    }
    if (g_str_has_prefix(desktop_id, "application://")) {
        return g_strdup(desktop_id);
    }
    const gchar *base = strrchr(desktop_id, '/');
    const gchar *name = (base != NULL && base[1] != '\0') ? (base + 1) : desktop_id;
    if (g_str_has_suffix(name, ".desktop")) {
        return g_strdup_printf("application://%s", name);
    }
    return g_strdup_printf("application://%s.desktop", name);
}

static guint fnv1a_hash(const gchar *s) {
    guint h = 2166136261u;
    for (const unsigned char *p = (const unsigned char *)s; *p != '\0'; ++p) {
        h ^= (guint)(*p);
        h *= 16777619u;
    }
    return h;
}

__attribute__((visibility("default")))
gpointer unity_launcher_entry_get_for_desktop_id(const gchar *desktop_id) {
    if (desktop_id == NULL || desktop_id[0] == '\0') {
        return NULL;
    }

    gchar *app_uri = normalize_app_uri(desktop_id);

    g_mutex_lock(&g_registry_lock);
    if (g_entries == NULL) {
        g_entries = g_hash_table_new_full(g_str_hash, g_str_equal, g_free, g_object_unref);
    }

    MalusUnityLauncherEntry *entry =
        (MalusUnityLauncherEntry *)g_hash_table_lookup(g_entries, app_uri);
    if (entry == NULL) {
        entry = (MalusUnityLauncherEntry *)g_object_new(
            malus_unity_launcher_entry_get_type(), NULL);
        entry->app_uri = g_strdup(app_uri);
        entry->object_path =
            g_strdup_printf("/com/canonical/unity/launcherentry/%u", fnv1a_hash(app_uri));
        g_hash_table_insert(g_entries, g_strdup(app_uri), entry);
    }
    g_mutex_unlock(&g_registry_lock);

    g_free(app_uri);
    return entry;
}

__attribute__((visibility("default")))
gpointer unity_launcher_entry_get_for_desktop_file(const gchar *desktop_file) {
    return unity_launcher_entry_get_for_desktop_id(desktop_file);
}

__attribute__((visibility("default")))
void unity_launcher_entry_set_count(gpointer entry_ptr, gint64 count) {
    if (entry_ptr == NULL) {
        return;
    }
    MalusUnityLauncherEntry *entry = (MalusUnityLauncherEntry *)entry_ptr;
    g_mutex_lock(&entry->lock);
    if (entry->count != count) {
        entry->count = count;
        emit_update_locked(entry);
    }
    g_mutex_unlock(&entry->lock);
}

__attribute__((visibility("default")))
gint64 unity_launcher_entry_get_count(gpointer entry_ptr) {
    if (entry_ptr == NULL) {
        return 0;
    }
    MalusUnityLauncherEntry *entry = (MalusUnityLauncherEntry *)entry_ptr;
    g_mutex_lock(&entry->lock);
    gint64 val = entry->count;
    g_mutex_unlock(&entry->lock);
    return val;
}

__attribute__((visibility("default")))
void unity_launcher_entry_set_count_visible(gpointer entry_ptr, gboolean visible) {
    if (entry_ptr == NULL) {
        return;
    }
    MalusUnityLauncherEntry *entry = (MalusUnityLauncherEntry *)entry_ptr;
    g_mutex_lock(&entry->lock);
    gboolean norm = visible ? TRUE : FALSE;
    if (entry->count_visible != norm) {
        entry->count_visible = norm;
        emit_update_locked(entry);
    }
    g_mutex_unlock(&entry->lock);
}

__attribute__((visibility("default")))
gboolean unity_launcher_entry_get_count_visible(gpointer entry_ptr) {
    if (entry_ptr == NULL) {
        return FALSE;
    }
    MalusUnityLauncherEntry *entry = (MalusUnityLauncherEntry *)entry_ptr;
    g_mutex_lock(&entry->lock);
    gboolean val = entry->count_visible;
    g_mutex_unlock(&entry->lock);
    return val;
}

__attribute__((visibility("default")))
void unity_launcher_entry_set_progress(gpointer entry_ptr, gdouble progress) {
    if (entry_ptr == NULL) {
        return;
    }
    if (isnan(progress)) {
        progress = 0.0;
    } else if (progress < 0.0) {
        progress = 0.0;
    } else if (progress > 1.0) {
        progress = 1.0;
    }

    MalusUnityLauncherEntry *entry = (MalusUnityLauncherEntry *)entry_ptr;
    g_mutex_lock(&entry->lock);
    entry->progress = progress;
    gint64 now_us = g_get_monotonic_time();
    gboolean boundary = (progress <= 0.005 || progress >= 0.995);
    gboolean significant = fabs(progress - entry->last_emitted_progress) >= 0.005;
    gboolean interval_ok = (now_us - entry->last_emit_us) >= 40000; /* 40ms */
    if (boundary || (significant && interval_ok)) {
        entry->last_emitted_progress = progress;
        entry->last_emit_us = now_us;
        emit_update_locked(entry);
    }
    g_mutex_unlock(&entry->lock);
}

__attribute__((visibility("default")))
gdouble unity_launcher_entry_get_progress(gpointer entry_ptr) {
    if (entry_ptr == NULL) {
        return 0.0;
    }
    MalusUnityLauncherEntry *entry = (MalusUnityLauncherEntry *)entry_ptr;
    g_mutex_lock(&entry->lock);
    gdouble val = entry->progress;
    g_mutex_unlock(&entry->lock);
    return val;
}

__attribute__((visibility("default")))
void unity_launcher_entry_set_progress_visible(gpointer entry_ptr, gboolean visible) {
    if (entry_ptr == NULL) {
        return;
    }
    MalusUnityLauncherEntry *entry = (MalusUnityLauncherEntry *)entry_ptr;
    g_mutex_lock(&entry->lock);
    gboolean norm = visible ? TRUE : FALSE;
    if (entry->progress_visible != norm) {
        entry->progress_visible = norm;
        entry->last_emitted_progress = entry->progress;
        entry->last_emit_us = g_get_monotonic_time();
        emit_update_locked(entry);
    }
    g_mutex_unlock(&entry->lock);
}

__attribute__((visibility("default")))
gboolean unity_launcher_entry_get_progress_visible(gpointer entry_ptr) {
    if (entry_ptr == NULL) {
        return FALSE;
    }
    MalusUnityLauncherEntry *entry = (MalusUnityLauncherEntry *)entry_ptr;
    g_mutex_lock(&entry->lock);
    gboolean val = entry->progress_visible;
    g_mutex_unlock(&entry->lock);
    return val;
}

__attribute__((visibility("default")))
void unity_launcher_entry_set_urgent(gpointer entry_ptr, gboolean urgent) {
    if (entry_ptr == NULL) {
        return;
    }
    MalusUnityLauncherEntry *entry = (MalusUnityLauncherEntry *)entry_ptr;
    g_mutex_lock(&entry->lock);
    gboolean norm = urgent ? TRUE : FALSE;
    if (entry->urgent != norm) {
        entry->urgent = norm;
        emit_update_locked(entry);
    }
    g_mutex_unlock(&entry->lock);
}

__attribute__((visibility("default")))
gboolean unity_launcher_entry_get_urgent(gpointer entry_ptr) {
    if (entry_ptr == NULL) {
        return FALSE;
    }
    MalusUnityLauncherEntry *entry = (MalusUnityLauncherEntry *)entry_ptr;
    g_mutex_lock(&entry->lock);
    gboolean val = entry->urgent;
    g_mutex_unlock(&entry->lock);
    return val;
}
