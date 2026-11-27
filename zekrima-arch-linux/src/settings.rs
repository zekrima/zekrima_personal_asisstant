//! Native GTK4 Settings window for Zekrima Personal Assistant on Arch Linux / CachyOS.
//!
//! Manages BYOK API keys for Google Gemini 3.8 Flash, OpenRouter (Free),
//! OpenCode Zen (Free), NVIDIA NIM (Free), Groq, and OpenAI/Custom, plus
//! Gemini TTS voice, Screen Vision resolution, Auto-Hide, and Hotkey preferences.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, Box as GtkBox, Button, CheckButton, DropDown, Entry, Label, Orientation,
    PasswordEntry, ScrolledWindow, StringList, Window,
};

use crate::services::assistant;
use crate::services::config::{
    AssistantConfig, ALL_HIDE_HOTKEYS, ALL_LLM_PROVIDERS, ALL_PTT_HOTKEYS, ALL_SCREEN_RESOLUTIONS,
    ALL_STT_PROVIDERS, CICADA_LANGUAGES, GEMINI_TTS_VOICES,
};

thread_local! {
    static SETTINGS_WIN: RefCell<Option<gtk4::glib::WeakRef<Window>>> = const { RefCell::new(None) };
}

pub fn open_settings_window<F>(app: &Application, on_saved: F)
where
    F: Fn() + 'static,
{
    let existing = SETTINGS_WIN.with(|slot| slot.borrow().as_ref().and_then(|w| w.upgrade()));
    if let Some(win) = existing {
        win.present();
        return;
    }

    let cfg = Rc::new(RefCell::new(AssistantConfig::load()));
    let active_provider_idx = Rc::new(RefCell::new(
        ALL_LLM_PROVIDERS
            .iter()
            .position(|&p| p == cfg.borrow().llm_provider)
            .unwrap_or(0),
    ));

    let (dyn_w, dyn_h) = gtk4::gdk::Display::default()
        .and_then(|d| {
            let mons = d.monitors();
            mons.item(0)
                .and_then(|o| o.downcast::<gtk4::gdk::Monitor>().ok())
                .map(|m| {
                    let g = m.geometry();
                    let w = ((g.width() as f32 * 0.36).clamp(420.0, 620.0) as i32)
                        .min((g.width() - 32).max(360));
                    let h = ((g.height() as f32 * 0.80).clamp(480.0, 820.0) as i32)
                        .min((g.height() - 64).max(400));
                    (w, h)
                })
        })
        .unwrap_or((520, 680));

    let win = Window::builder()
        .application(app)
        .title("Zekrima Assistant — Settings (BYOK)")
        .default_width(dyn_w)
        .default_height(dyn_h)
        .resizable(true)
        .build();

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let root = GtkBox::new(Orientation::Vertical, 10);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(18);
    root.set_margin_end(18);

    // Header
    let title = Label::new(Some("Zekrima Personal Assistant Settings"));
    title.set_halign(Align::Start);
    title.add_css_class("title-3");
    root.append(&title);

    let subtitle = Label::new(Some(
        "BYOK lokal (~/.config/zekrima/assistant.ini, izin 0600). Mendukung model gratis Gemini 3.8 Flash, OpenRouter, OpenCode Zen, NVIDIA NIM & Groq.",
    ));
    subtitle.set_halign(Align::Start);
    subtitle.set_wrap(true);
    subtitle.set_xalign(0.0);
    subtitle.add_css_class("dim-label");
    root.append(&subtitle);

    // 1. LLM Provider
    root.append(&section_label("AI Provider (LLM)"));
    let provider_names: Vec<&str> = ALL_LLM_PROVIDERS.iter().map(|p| p.display_name()).collect();
    let provider_dd = DropDown::from_strings(&provider_names);
    provider_dd.set_selected(*active_provider_idx.borrow() as u32);
    root.append(&provider_dd);

    // 2. Active Provider API Key
    let key_hint_lbl = section_label(cfg.borrow().llm_provider.key_hint());
    root.append(&key_hint_lbl);
    let api_key_entry = PasswordEntry::new();
    api_key_entry.set_show_peek_icon(true);
    api_key_entry.set_text(cfg.borrow().api_key_for(cfg.borrow().llm_provider));
    root.append(&api_key_entry);

    // 3. Live Auto-Detected Model Picker & Editable Model ID
    root.append(&section_label(
        "Model (otomatis terdeteksi dari API saat isi key, atau ketik custom)",
    ));
    let initial_presets: Vec<String> = cfg
        .borrow()
        .llm_provider
        .model_presets()
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let available_models = Rc::new(RefCell::new(initial_presets.clone()));
    let preset_strs: Vec<&str> = initial_presets.iter().map(|s| s.as_str()).collect();
    let preset_dd = DropDown::from_strings(&preset_strs);
    preset_dd.set_hexpand(true);
    let current_model = cfg.borrow().model_for(cfg.borrow().llm_provider).to_string();
    if let Some(pos) = initial_presets.iter().position(|m| m == &current_model) {
        preset_dd.set_selected(pos as u32);
    }

    let fetch_models_btn = Button::with_label("↻ Cek Model");
    let model_row = GtkBox::new(Orientation::Horizontal, 6);
    model_row.append(&preset_dd);
    model_row.append(&fetch_models_btn);
    root.append(&model_row);

    let model_status_lbl = Label::new(Some(
        "Isi API key untuk mendeteksi daftar model tersedia secara otomatis.",
    ));
    model_status_lbl.set_halign(Align::Start);
    model_status_lbl.set_wrap(true);
    model_status_lbl.set_xalign(0.0);
    model_status_lbl.add_css_class("dim-label");
    root.append(&model_status_lbl);

    let model_entry = Entry::new();
    model_entry.set_text(&current_model);
    model_entry.set_placeholder_text(Some("Model ID (contoh: gemini-3.8-flash / openrouter/free)"));
    root.append(&model_entry);

    // 4. OpenAI / Custom Base URL
    root.append(&section_label("Base URL (khusus OpenAI / Custom / Ollama)"));
    let base_url_entry = Entry::new();
    base_url_entry.set_text(&cfg.borrow().openai_base_url);
    base_url_entry.set_sensitive(cfg.borrow().llm_provider == crate::services::config::LlmProvider::OpenAi);
    root.append(&base_url_entry);

    // 5. Voice STT Provider & Groq Key
    root.append(&section_label("Voice STT Provider"));
    let stt_names: Vec<&str> = ALL_STT_PROVIDERS.iter().map(|s| s.display_name()).collect();
    let stt_dd = DropDown::from_strings(&stt_names);
    let stt_idx = ALL_STT_PROVIDERS
        .iter()
        .position(|&s| s == cfg.borrow().stt_provider)
        .unwrap_or(0);
    stt_dd.set_selected(stt_idx as u32);
    root.append(&stt_dd);

    root.append(&section_label("Groq API Key (Opsional untuk Whisper STT Gratis <300ms)"));
    let groq_key_entry = PasswordEntry::new();
    groq_key_entry.set_show_peek_icon(true);
    groq_key_entry.set_text(&cfg.borrow().groq_api_key);
    root.append(&groq_key_entry);

    // 6. Gemini TTS Enabled & Voice
    let tts_check = CheckButton::with_label("Aktifkan Suara TTS (Gemini 3.8 Flash TTS via PipeWire)");
    tts_check.set_active(cfg.borrow().gemini_tts_enabled);
    root.append(&tts_check);

    root.append(&section_label("Suara TTS Gemini"));
    let voice_dd = DropDown::from_strings(&GEMINI_TTS_VOICES);
    let voice_idx = GEMINI_TTS_VOICES
        .iter()
        .position(|&v| v.eq_ignore_ascii_case(&cfg.borrow().gemini_tts_voice))
        .unwrap_or(0);
    voice_dd.set_selected(voice_idx as u32);
    root.append(&voice_dd);

    // 7. Screen Vision Resolution
    root.append(&section_label("Resolusi Screen Vision (Zero Retention)"));
    let res_names: Vec<&str> = ALL_SCREEN_RESOLUTIONS
        .iter()
        .map(|r| r.display_name())
        .collect();
    let res_dd = DropDown::from_strings(&res_names);
    let res_idx = ALL_SCREEN_RESOLUTIONS
        .iter()
        .position(|&r| r == cfg.borrow().screen_resolution)
        .unwrap_or(0);
    res_dd.set_selected(res_idx as u32);
    root.append(&res_dd);

    // 8. Auto-Hide & Hotkey preferences
    let auto_hide_check = CheckButton::with_label(
        "Auto-Hide ke tepi atas saat idle 10 detik (arahkan mouse ke atas untuk muncul)",
    );
    auto_hide_check.set_active(cfg.borrow().auto_hide);
    root.append(&auto_hide_check);

    root.append(&section_label("Push-to-Talk Hotkey (Wayland bind: `zekrima --voice`)"));
    let ptt_names: Vec<&str> = ALL_PTT_HOTKEYS.iter().map(|h| h.label()).collect();
    let ptt_dd = DropDown::from_strings(&ptt_names);
    let ptt_idx = ALL_PTT_HOTKEYS
        .iter()
        .position(|&h| h == cfg.borrow().ptt_hotkey)
        .unwrap_or(0);
    ptt_dd.set_selected(ptt_idx as u32);
    root.append(&ptt_dd);

    root.append(&section_label("Hide/Show Hotkey (Wayland bind: `zekrima --toggle-hide`)"));
    let hide_names: Vec<&str> = ALL_HIDE_HOTKEYS.iter().map(|h| h.label()).collect();
    let hide_dd = DropDown::from_strings(&hide_names);
    let hide_idx = ALL_HIDE_HOTKEYS
        .iter()
        .position(|&h| h == cfg.borrow().hide_hotkey)
        .unwrap_or(0);
    hide_dd.set_selected(hide_idx as u32);
    root.append(&hide_dd);

    // 9. Language
    root.append(&section_label("Bahasa (id / en / auto)"));
    let lang_entry = Entry::new();
    lang_entry.set_text(&cfg.borrow().language);
    root.append(&lang_entry);

    // 10. Cicada Mode (Live Meeting Translator)
    root.append(&section_label("Cicada Mode — Bahasa Target Terjemahan"));
    let cicada_lang_labels: Vec<&str> = CICADA_LANGUAGES.iter().map(|(_, l)| *l).collect();
    let cicada_target_dd = DropDown::from_strings(&cicada_lang_labels);
    let ct_idx = CICADA_LANGUAGES
        .iter()
        .position(|(code, _)| code.eq_ignore_ascii_case(&cfg.borrow().cicada_target_lang))
        .unwrap_or(0);
    cicada_target_dd.set_selected(ct_idx as u32);
    root.append(&cicada_target_dd);

    root.append(&section_label("Cicada Mode — Bahasa Quick-Reply (Bidirectional)"));
    let cicada_reply_dd = DropDown::from_strings(&cicada_lang_labels);
    let cr_idx = CICADA_LANGUAGES
        .iter()
        .position(|(code, _)| code.eq_ignore_ascii_case(&cfg.borrow().cicada_reply_lang))
        .unwrap_or(1);
    cicada_reply_dd.set_selected(cr_idx as u32);
    root.append(&cicada_reply_dd);

    root.append(&section_label("Cicada Custom Glossary (pisahkan dengan ; contoh: Zekrima; Kubernetes)"));
    let cicada_glossary_entry = Entry::new();
    cicada_glossary_entry.set_text(&cfg.borrow().cicada_glossary);
    cicada_glossary_entry.set_placeholder_text(Some("Istilah teknis / nama produk / singkatan..."));
    root.append(&cicada_glossary_entry);

    // Helper for non-blocking live API model discovery
    let fetch_gen = Rc::new(Cell::new(0u64));
    let debounce_gen = Rc::new(Cell::new(0u64));
    let suppress_edit = Rc::new(Cell::new(false));

    let trigger_fetch: Rc<dyn Fn(bool)> = {
        let idx_c = Rc::clone(&active_provider_idx);
        let api_key_c = api_key_entry.clone();
        let base_url_c = base_url_entry.clone();
        let preset_dd_c = preset_dd.clone();
        let model_entry_c = model_entry.clone();
        let status_c = model_status_lbl.clone();
        let avail_c = Rc::clone(&available_models);
        let fgen_c = Rc::clone(&fetch_gen);

        Rc::new(move |manual: bool| {
            let provider = ALL_LLM_PROVIDERS[*idx_c.borrow()];
            let key = api_key_c.text().trim().to_string();
            let base = base_url_c.text().trim().to_string();
            if key.is_empty()
                && !matches!(
                    provider,
                    crate::services::config::LlmProvider::OpenRouter
                        | crate::services::config::LlmProvider::OpenAi
                )
            {
                status_c.set_text(if manual {
                    "Masukkan API key terlebih dahulu untuk mendeteksi model dari API."
                } else {
                    "Preset bawaan aktif — isi API key untuk auto-deteksi semua model API."
                });
                return;
            }

            let gen = fgen_c.get().wrapping_add(1);
            fgen_c.set(gen);
            status_c.set_text(&format!(
                "⏳ Mendeteksi daftar model dari {}...",
                provider.short_badge()
            ));

            let dd_weak = preset_dd_c.downgrade();
            let entry_weak = model_entry_c.downgrade();
            let status_weak = status_c.downgrade();
            let avail_inner = Rc::clone(&avail_c);
            let fgen_inner = Rc::clone(&fgen_c);
            let idx_inner = Rc::clone(&idx_c);

            // Spawn HTTP fetch on worker thread, deliver result back via MainContext::invoke
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = thread::Builder::new()
                .name("zekrima-fetch-models".to_owned())
                .spawn(move || {
                    let res = assistant::fetch_available_models(provider, &key, &base);
                    let _ = tx.send(res);
                });

            glib::timeout_add_local(Duration::from_millis(80), move || {
                let res = match rx.try_recv() {
                    Ok(r) => r,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        return glib::ControlFlow::Continue;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        return glib::ControlFlow::Break;
                    }
                };

                if fgen_inner.get() != gen || ALL_LLM_PROVIDERS[*idx_inner.borrow()] != provider {
                    return glib::ControlFlow::Break;
                }
                let (Some(dd), Some(entry), Some(st_lbl)) = (
                    dd_weak.upgrade(),
                    entry_weak.upgrade(),
                    status_weak.upgrade(),
                ) else {
                    return glib::ControlFlow::Break;
                };

                match res {
                    Ok(models) => {
                        let count = models.len();
                        let cur = entry.text().trim().to_string();
                        let strs: Vec<&str> = models.iter().map(|s| s.as_str()).collect();
                        let list = StringList::new(&strs);
                        *avail_inner.borrow_mut() = models.clone();
                        dd.set_model(Some(&list));
                        if let Some(pos) = models.iter().position(|m| m == &cur) {
                            dd.set_selected(pos as u32);
                        } else if let Some(first) = models.first() {
                            dd.set_selected(0);
                            entry.set_text(first);
                        }
                        st_lbl.set_text(&format!(
                            "✓ {count} model terdeteksi otomatis dari {} — pilih di dropdown!",
                            provider.short_badge()
                        ));
                    }
                    Err(err) => {
                        st_lbl.set_text(&format!("! {err} (preset bawaan tetap bisa dipakai)"));
                    }
                }
                glib::ControlFlow::Break
            });
        })
    };

    // Wiring: switching provider preserves previous provider's key/model & updates presets
    {
        let cfg_c = Rc::clone(&cfg);
        let idx_c = Rc::clone(&active_provider_idx);
        let key_hint_c = key_hint_lbl.clone();
        let api_key_c = api_key_entry.clone();
        let preset_dd_c = preset_dd.clone();
        let model_entry_c = model_entry.clone();
        let base_url_c = base_url_entry.clone();
        let groq_key_c = groq_key_entry.clone();
        let avail_c = Rc::clone(&available_models);
        let sup_c = Rc::clone(&suppress_edit);
        let fetch_c = Rc::clone(&trigger_fetch);

        provider_dd.connect_selected_notify(move |dd| {
            let new_idx = (dd.selected() as usize).min(ALL_LLM_PROVIDERS.len() - 1);
            let old_idx = *idx_c.borrow();
            if old_idx == new_idx {
                return;
            }
            let old_provider = ALL_LLM_PROVIDERS[old_idx];
            let new_provider = ALL_LLM_PROVIDERS[new_idx];

            {
                let mut c = cfg_c.borrow_mut();
                c.set_api_key_for(old_provider, api_key_c.text().to_string());
                c.set_model_for(old_provider, model_entry_c.text().to_string());
                if old_provider == crate::services::config::LlmProvider::Groq {
                    groq_key_c.set_text(&c.groq_api_key);
                }
                if old_provider == crate::services::config::LlmProvider::OpenAi {
                    let bu = base_url_c.text().trim().trim_end_matches('/').to_string();
                    if !bu.is_empty() {
                        c.openai_base_url = bu;
                    }
                }
                c.llm_provider = new_provider;
            }
            *idx_c.borrow_mut() = new_idx;

            sup_c.set(true);
            {
                let c = cfg_c.borrow();
                key_hint_c.set_text(new_provider.key_hint());
                api_key_c.set_text(c.api_key_for(new_provider));
                let presets = new_provider.model_presets();
                *avail_c.borrow_mut() = presets.iter().map(|s| (*s).to_string()).collect();
                let list = StringList::new(presets);
                preset_dd_c.set_model(Some(&list));
                let cur_model = c.model_for(new_provider);
                model_entry_c.set_text(cur_model);
                if let Some(pos) = presets.iter().position(|&m| m == cur_model) {
                    preset_dd_c.set_selected(pos as u32);
                }
                base_url_c.set_sensitive(new_provider == crate::services::config::LlmProvider::OpenAi);
                if new_provider == crate::services::config::LlmProvider::OpenAi {
                    base_url_c.set_text(&c.openai_base_url);
                } else {
                    base_url_c.set_text(new_provider.default_base_url());
                }
            }
            sup_c.set(false);
            fetch_c(false);
        });
    }

    // Wiring: selecting a detected/preset model updates model_entry
    {
        let avail_c = Rc::clone(&available_models);
        let model_entry_c = model_entry.clone();
        preset_dd.connect_selected_notify(move |dd| {
            let sel = dd.selected() as usize;
            if let Some(m) = avail_c.borrow().get(sel) {
                model_entry_c.set_text(m);
            }
        });
    }

    // Wiring: manual "Cek Model" button + debounced auto-detect when editing API Key / Base URL
    {
        let fetch_c = Rc::clone(&trigger_fetch);
        fetch_models_btn.connect_clicked(move |_| fetch_c(true));
    }
    {
        let sup_c = Rc::clone(&suppress_edit);
        let dgen_c = Rc::clone(&debounce_gen);
        let fetch_c = Rc::clone(&trigger_fetch);
        api_key_entry.connect_changed(move |_| {
            if sup_c.get() {
                return;
            }
            let next = dgen_c.get().wrapping_add(1);
            dgen_c.set(next);
            let dgen_inner = Rc::clone(&dgen_c);
            let fetch_inner = Rc::clone(&fetch_c);
            glib::timeout_add_local_once(Duration::from_millis(650), move || {
                if dgen_inner.get() == next {
                    fetch_inner(false);
                }
            });
        });
    }
    {
        let sup_c = Rc::clone(&suppress_edit);
        let dgen_c = Rc::clone(&debounce_gen);
        let fetch_c = Rc::clone(&trigger_fetch);
        base_url_entry.connect_changed(move |_| {
            if sup_c.get() {
                return;
            }
            let next = dgen_c.get().wrapping_add(1);
            dgen_c.set(next);
            let dgen_inner = Rc::clone(&dgen_c);
            let fetch_inner = Rc::clone(&fetch_c);
            glib::timeout_add_local_once(Duration::from_millis(650), move || {
                if dgen_inner.get() == next {
                    fetch_inner(false);
                }
            });
        });
    }

    // Auto-detect on initial open if key is already present
    trigger_fetch(false);

    // Action buttons row
    let btn_row = GtkBox::new(Orientation::Horizontal, 8);
    btn_row.set_margin_top(10);
    btn_row.set_halign(Align::End);

    let open_ini_btn = Button::with_label("Open INI");
    open_ini_btn.connect_clicked(|_| {
        AssistantConfig::open_in_editor();
    });

    let cancel_btn = Button::with_label("Cancel");
    {
        let win_c = win.clone();
        cancel_btn.connect_clicked(move |_| win_c.close());
    }

    let save_btn = Button::with_label("Save & Apply");
    save_btn.add_css_class("suggested-action");
    {
        let cfg_c = Rc::clone(&cfg);
        let idx_c = Rc::clone(&active_provider_idx);
        let win_c = win.clone();
        save_btn.connect_clicked(move |_| {
            let mut c = cfg_c.borrow_mut();
            let active_p = ALL_LLM_PROVIDERS[*idx_c.borrow()];
            c.llm_provider = active_p;
            c.set_api_key_for(active_p, api_key_entry.text().to_string());
            c.set_model_for(active_p, model_entry.text().to_string());

            let g_key = groq_key_entry.text().trim().to_string();
            if active_p != crate::services::config::LlmProvider::Groq && !g_key.is_empty() {
                c.groq_api_key = g_key;
            } else if active_p != crate::services::config::LlmProvider::Groq && g_key.is_empty() {
                c.groq_api_key.clear();
            }

            if active_p == crate::services::config::LlmProvider::OpenAi {
                let bu = base_url_entry.text().trim().trim_end_matches('/').to_string();
                if !bu.is_empty() {
                    c.openai_base_url = bu;
                }
            }

            c.stt_provider =
                ALL_STT_PROVIDERS[(stt_dd.selected() as usize).min(ALL_STT_PROVIDERS.len() - 1)];
            c.gemini_tts_enabled = tts_check.is_active();
            c.gemini_tts_voice = GEMINI_TTS_VOICES
                [(voice_dd.selected() as usize).min(GEMINI_TTS_VOICES.len() - 1)]
            .to_string();
            c.screen_resolution = ALL_SCREEN_RESOLUTIONS
                [(res_dd.selected() as usize).min(ALL_SCREEN_RESOLUTIONS.len() - 1)];
            c.auto_hide = auto_hide_check.is_active();
            c.ptt_hotkey =
                ALL_PTT_HOTKEYS[(ptt_dd.selected() as usize).min(ALL_PTT_HOTKEYS.len() - 1)];
            c.hide_hotkey =
                ALL_HIDE_HOTKEYS[(hide_dd.selected() as usize).min(ALL_HIDE_HOTKEYS.len() - 1)];

            let lang = lang_entry.text().trim().to_string();
            c.language = if lang.is_empty() {
                "id".to_string()
            } else {
                lang
            };

            c.cicada_target_lang = CICADA_LANGUAGES
                [(cicada_target_dd.selected() as usize).min(CICADA_LANGUAGES.len() - 1)]
            .0
            .to_string();
            c.cicada_reply_lang = CICADA_LANGUAGES
                [(cicada_reply_dd.selected() as usize).min(CICADA_LANGUAGES.len() - 1)]
            .0
            .to_string();
            c.cicada_glossary = cicada_glossary_entry.text().trim().to_string();

            let _ = c.save();
            drop(c);
            on_saved();
            win_c.close();
        });
    }

    btn_row.append(&open_ini_btn);
    btn_row.append(&cancel_btn);
    btn_row.append(&save_btn);
    root.append(&btn_row);

    scroll.set_child(Some(&root));
    win.set_child(Some(&scroll));

    SETTINGS_WIN.with(|slot| {
        *slot.borrow_mut() = Some(win.downgrade());
    });
    win.present();
}

fn section_label(text: &str) -> Label {
    let lbl = Label::new(Some(text));
    lbl.set_halign(Align::Start);
    lbl.set_xalign(0.0);
    lbl.set_margin_top(4);
    lbl
}
