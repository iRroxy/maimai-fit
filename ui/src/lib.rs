use std::{borrow::Cow, cell::RefCell, rc::Rc};

use js_sys::Array;
use serde::Serialize;
use serde_json::{json, Value};
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{
    Blob, BlobPropertyBag, Element, Event, EventTarget, HtmlAnchorElement,
    HtmlInputElement, HtmlSelectElement, HtmlTextAreaElement, Request, RequestCache,
    RequestCredentials, RequestInit, Response, Url,
};

const DEMO_ANCHOR: &str = "示例鸡肉主食（一份）";
type Shared = Rc<RefCell<State>>;

#[derive(Default)]
struct State {
    catalog: Option<Rc<Value>>,
    startup: Option<Rc<Value>>,
    selected: Vec<String>,
    result: Option<Value>,
    revision: u64,
    importing: u64,
    busy: bool,
    online_busy: bool,
}

fn window() -> web_sys::Window {
    web_sys::window().expect("browser window")
}

fn document() -> web_sys::Document {
    window().document().expect("browser document")
}

fn element(id: &str) -> Element {
    document().get_element_by_id(id).expect("UI element")
}

fn input(id: &str) -> HtmlInputElement {
    element(id).unchecked_into()
}

fn text(id: &str, value: &str) {
    element(id).set_text_content(Some(value));
}

fn attr(element: &Element, name: &str, enabled: bool) {
    if enabled {
        element.set_attribute(name, "").expect("DOM attribute");
    } else {
        element.remove_attribute(name).expect("DOM attribute");
    }
}

fn hidden(id: &str, value: bool) {
    attr(&element(id), "hidden", value);
}

fn disabled(id: &str, value: bool) {
    attr(&element(id), "disabled", value);
}

fn node(tag: &str, value: &str, class: &str) -> Element {
    let result = document().create_element(tag).expect("DOM element");
    result.set_text_content(Some(value));
    if !class.is_empty() {
        result.set_class_name(class);
    }
    result
}

fn append(parent: &Element, child: &Element) {
    parent.append_child(child).expect("DOM child");
}

fn clear(id: &str) -> Element {
    let result = element(id);
    result.set_text_content(None);
    result
}

fn message(value: &str, error: bool, target: &str) {
    let target = element(target);
    target.set_text_content(Some(value));
    target.class_list().toggle_with_force("error", error).expect("DOM class");
    attr(&target, "hidden", value.is_empty());
}

fn dismiss_message() {
    message("", false, "message");
}

fn display(value: &Value) -> Cow<'_, str> {
    match value {
        Value::String(value) => Cow::Borrowed(value),
        Value::Null => Cow::Borrowed(""),
        _ => Cow::Owned(value.to_string()),
    }
}

fn foods(catalog: &Value) -> &[Value] {
    catalog["foods"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn same_catalog(left: &Option<Rc<Value>>, right: &Option<Rc<Value>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Rc::ptr_eq(left, right),
        (None, None) => true,
        _ => false,
    }
}

fn invalidate(state: &mut State) {
    state.revision += 1;
    state.result = None;
    hidden("download", true);
    hidden("export-note", true);
    hidden("ranking", true);
    text("result-summary", if state.catalog.is_some() {
        "条件已更新，请生成搭配。"
    } else {
        "请先获取官方营养数据或导入目录。"
    });
    let empty = node("div", "", "empty-state");
    append(&empty, &node("span", "＋", "empty-symbol"));
    append(&empty, &node("h3", "从喜欢的那一份开始", ""));
    append(&empty, &node("p", "选择保留餐品并设定条件，生成符合全部限制的搭配。", ""));
    append(&clear("results"), &empty);
}

fn render_selected(state: &State) {
    text("selected-count", &state.selected.len().to_string());
    disabled("clear", state.online_busy || state.selected.is_empty());
    let list = clear("selected-list");
    for name in &state.selected {
        let chip = node("button", &format!("{name} ×"), "selected-chip");
        chip.set_attribute("type", "button").expect("button type");
        chip.set_attribute("data-food", name).expect("food attribute");
        chip.set_attribute("aria-label", &format!("取消保留 {name}")).expect("button label");
        attr(&chip, "disabled", state.online_busy);
        append(&list, &chip);
    }
    let checkboxes = element("food-list").query_selector_all("input").expect("food inputs");
    for index in 0..checkboxes.length() {
        if let Some(checkbox) = checkboxes.item(index).and_then(|node| node.dyn_into::<HtmlInputElement>().ok()) {
            let selected = state.selected.contains(&checkbox.value());
            checkbox.set_checked(selected);
            checkbox.set_disabled(state.online_busy || (!selected && state.selected.len() >= 3));
        }
    }
}

fn select_food(state: &mut State, name: String, checked: bool) {
    if state.online_busy {
        return;
    }
    if checked {
        if state.selected.len() >= 3 || state.selected.contains(&name) {
            render_selected(state);
            return;
        }
        if !state.catalog.as_ref().is_some_and(|catalog| foods(catalog).iter().any(|food| food["name"].as_str() == Some(&name))) {
            return;
        }
        state.selected.push(name);
    } else {
        state.selected.retain(|selected| selected != &name);
    }
    dismiss_message();
    invalidate(state);
    render_selected(state);
}

fn render_foods(state: &State) {
    let query = input("search").value().trim().to_lowercase();
    let list = clear("food-list");
    let mut count = 0;
    if let Some(catalog) = &state.catalog {
        for food in foods(catalog) {
            let name = food["name"].as_str().unwrap_or("");
            if !query.is_empty() && !name.to_lowercase().contains(&query) {
                continue;
            }
            let row = node("label", "", "food-option");
            let checkbox: HtmlInputElement = node("input", "", "").unchecked_into();
            checkbox.set_type("checkbox");
            checkbox.set_value(name);
            let selected = state.selected.iter().any(|selected| selected == name);
            checkbox.set_checked(selected);
            checkbox.set_disabled(state.online_busy || (!selected && state.selected.len() >= 3));
            let description = node("span", "", "food-description");
            append(&description, &node("span", name, "food-name"));
            let sodium = if food["sodium_mg"].is_null() {
                "未知".to_owned()
            } else {
                format!("{} mg", display(&food["sodium_mg"]))
            };
            append(&description, &node("span", &format!("{} kcal · 蛋白质 {} g · 钠 {sodium}", display(&food["kcal"]), display(&food["protein_g"])), "food-nutrients"));
            append(&row, checkbox.as_ref());
            append(&row, &description);
            append(&list, &row);
            count += 1;
        }
    }
    if count == 0 {
        append(&list, &node("p", if state.catalog.is_some() {
            "没有匹配的餐品；已保留项不受搜索影响。"
        } else {
            "请先在线获取营养，或导入已有目录。"
        }, "no-match"));
    }
    list.set_attribute("aria-busy", "false").expect("food busy state");
}

fn update_controls(state: &State) {
    disabled("online-fields", state.online_busy);
    disabled("planner-fields", state.online_busy || state.catalog.is_none());
    disabled("generate", state.busy || state.online_busy || state.catalog.is_none());
    disabled("catalog-file", state.online_busy);
    disabled("restore", state.online_busy || same_catalog(&state.catalog, &state.startup));
    disabled("search", state.online_busy || state.catalog.is_none());
    render_selected(state);
}

fn apply_catalog(state: &mut State, catalog: Option<Rc<Value>>, startup: bool, live: bool) {
    state.catalog = catalog;
    state.selected.clear();
    if startup && state.catalog.as_ref().is_some_and(|catalog| {
        catalog["source"]["kind"] == "demo" && foods(catalog).iter().any(|food| food["name"] == DEMO_ANCHOR)
    }) {
        state.selected.push(DEMO_ANCHOR.to_owned());
    }
    input("search").set_value("");
    text("restore", if state.startup.is_some() { "恢复启动目录" } else { "清除当前目录" });
    text("food-count", &state.catalog.as_ref().map(|catalog| format!("{} 种餐品", foods(catalog).len())).unwrap_or_else(|| "尚未连接".to_owned()));
    let source = state.catalog.as_ref().map(|catalog| &catalog["source"]);
    let kind = source.and_then(|source| source["kind"].as_str()).unwrap_or("");
    text("source-kind", if live {
        "本次在线获取 · 官方 MCP"
    } else {
        match kind {
            "demo" => "合成演示目录",
            "mcp" => "MCP 营养目录（文件声明）",
            "import" => "导入目录（来源由文件提供）",
            _ => "尚未获取官方营养",
        }
    });
    let description = if let Some(source) = source {
        if live {
            format!("获取时间：{}；不是实时菜单、价格或库存。", display(&source["fetched_at"]))
        } else if kind == "demo" {
            "合成名称与数值，不能用于真实点餐。".to_owned()
        } else {
            "离线目录，不代表本次在线查询或实时菜单。".to_owned()
        }
    } else {
        "获取或导入目录后，选择精确餐品与规格。".to_owned()
    };
    text("source-description", &description);
    let mut notes = Vec::new();
    if let Some(source) = source {
        for (key, prefix) in [("fetched_at", "采集时间："), ("tool", "工具："), ("note", "")] {
            let value = display(&source[key]);
            if !value.is_empty() {
                notes.push(format!("{prefix}{value}"));
            }
        }
    }
    text("source-note", &notes.join("\n"));
    hidden("source-details", notes.is_empty());
    attr(&element("source-details"), "open", false);
    invalidate(state);
    render_foods(state);
    update_controls(state);
}

fn prepare_request(path: &str, body: Option<&str>) -> Result<Request, String> {
    let options = RequestInit::new();
    options.set_cache(RequestCache::NoStore);
    options.set_credentials(RequestCredentials::Omit);
    if let Some(body) = body {
        options.set_method("POST");
        options.set_body(&JsValue::from_str(body));
    }
    let request = Request::new_with_str_and_init(path, &options).map_err(|_| "无法创建本地请求。".to_owned())?;
    if body.is_some() {
        request.headers().set("Content-Type", "application/json").map_err(|_| "无法创建本地请求。".to_owned())?;
    }
    Ok(request)
}

async fn api(request: Result<Request, String>) -> Result<Value, String> {
    let request = request?;
    let response = JsFuture::from(window().fetch_with_request(&request)).await
        .map_err(|_| "无法连接本地服务，请确认终端中的 serve 命令仍在运行。".to_owned())?
        .dyn_into::<Response>().map_err(|_| "本地服务未返回有效响应。".to_owned())?;
    let raw = JsFuture::from(response.text().map_err(|_| "无法读取本地服务响应。".to_owned())?).await
        .map_err(|_| "无法读取本地服务响应。".to_owned())?
        .as_string().ok_or_else(|| "本地服务未返回有效 JSON，请检查终端输出。".to_owned())?;
    let data: Value = serde_json::from_str(&raw).map_err(|_| "本地服务未返回有效 JSON，请检查终端输出。".to_owned())?;
    if !response.ok() {
        return Err(data["error"].as_str().unwrap_or("请求失败，请检查输入与本地服务。").to_owned());
    }
    Ok(data)
}

fn online_request(shared: &Shared, inspect: bool) {
    let token_input = input("mcp-token");
    let valid = token_input.report_validity();
    let token = token_input.value().trim().to_owned();
    token_input.set_value("");
    if shared.borrow().online_busy || !valid {
        return;
    }
    if token.is_empty() {
        message("请输入有效的 MCP Token。", true, "online-status");
        return;
    }
    // Build the browser request synchronously. Credentials never enter State or an async snapshot.
    let request = {
        let body = if inspect {
            json!({ "token": token })
        } else {
            let arguments: HtmlTextAreaElement = element("online-arguments").unchecked_into();
            json!({ "token": token, "arguments": arguments.value() })
        };
        prepare_request(if inspect { "/api/inspect" } else { "/api/fetch" }, Some(&body.to_string()))
    };
    drop(token);
    let generation = {
        let mut state = shared.borrow_mut();
        state.importing += 1;
        state.online_busy = true;
        if !inspect {
            invalidate(&mut state);
        }
        update_controls(&state);
        state.importing
    };
    dismiss_message();
    if inspect {
        hidden("tool-definition", true);
        text("tool-schema", "");
    }
    message(if inspect { "正在读取官方工具参数定义…" } else { "正在连接官方服务并获取营养目录…" }, false, "online-status");
    let shared = shared.clone();
    spawn_local(async move {
        let result = api(request).await;
        input("mcp-token").set_value("");
        let mut state = shared.borrow_mut();
        if generation == state.importing {
            match result {
                Ok(data) if inspect => {
                    text("tool-schema", &serde_json::to_string_pretty(&data).expect("JSON value"));
                    hidden("tool-definition", false);
                    attr(&element("tool-definition"), "open", true);
                    message("已读取当次参数定义。请按定义填写参数，再重新输入 Token 获取营养。", false, "online-status");
                }
                Ok(data) => {
                    let count = foods(&data).len();
                    apply_catalog(&mut state, Some(Rc::new(data)), false, true);
                    message(&format!("已从官方 MCP 获取 {count} 项营养数据，请选择保留餐品。"), false, "online-status");
                }
                Err(error) => {
                    message(&format!("{}：{error} 原目录未更改，未使用演示数据替代。", if inspect { "参数读取失败" } else { "在线获取失败" }), true, "online-status");
                }
            }
        }
        state.online_busy = false;
        update_controls(&state);
    });
}

fn metric(value: &Value, unit: &str, label: &str) -> Element {
    let metric = node("span", "", "metric");
    let value_text = if value.is_null() { Cow::Borrowed("未知") } else { display(value) };
    append(&metric, &node("strong", &value_text, ""));
    append(&metric, &node("span", &if value.is_null() { label.to_owned() } else { format!("{unit} {label}") }, ""));
    metric
}

fn render_result(result: &Value) {
    let list = clear("results");
    let plans = result["plans"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if result["status"] == "infeasible" {
        let empty = node("div", "", "empty-state");
        append(&empty, &node("h3", "没有符合全部条件的组合", ""));
        append(&empty, &node("p", "未替换保留餐品，也未放宽任何限制。请自行决定是否调整条件，再重新生成。", ""));
        let rejected = &result["search"]["unknown_sodium_rejections"];
        if rejected.as_u64().is_some_and(|value| value > 0) {
            append(&empty, &node("p", &format!("{} 个营养条件匹配的组合因钠数据缺失，无法核验。", display(rejected)), ""));
        }
        append(&list, &empty);
    } else {
        for (index, plan) in plans.iter().enumerate() {
            let card = node("details", "", "plan-card");
            attr(&card, "open", index == 0);
            let summary = node("summary", "", "");
            let title = node("span", &format!("方案 {:02}", index + 1), "plan-title");
            append(&title, &node("small", &format!("新增 {} 种", display(&plan["added_items"])), ""));
            let metrics = node("span", "", "plan-metrics");
            for (key, unit, label) in [("kcal", "kcal", "热量"), ("protein_g", "g", "蛋白质"), ("sodium_mg", "mg", "钠")] {
                append(&metrics, &metric(&plan["totals"][key], unit, label));
            }
            append(&summary, &title);
            append(&summary, &metrics);
            let content = node("div", "", "plan-content");
            let scroll = node("div", "", "table-scroll");
            let table = node("table", "", "");
            append(&table, &node("caption", &format!("方案 {} 逐项营养，每种餐品一份", index + 1), "sr-only"));
            let head = node("thead", "", "");
            let headings = node("tr", "", "");
            for label in ["餐品 · 每种一份", "kcal", "蛋白质 g", "钠 mg"] {
                let cell = node("th", label, "");
                cell.set_attribute("scope", "col").expect("table scope");
                append(&headings, &cell);
            }
            append(&head, &headings);
            append(&table, &head);
            let body = node("tbody", "", "");
            for food in foods(plan) {
                let row = node("tr", "", "");
                let food_name = display(&food["name"]);
                let name = node("td", &food_name, "");
                if result["constraints"]["keep"].as_array().is_some_and(|keep| keep.iter().any(|keep| {
                    keep.as_str().is_some_and(|keep| keep.trim().to_lowercase() == food_name.to_lowercase())
                })) {
                    append(&name, &node("span", "保留", "keep-tag"));
                }
                append(&row, &name);
                for key in ["kcal", "protein_g", "sodium_mg"] {
                    let value = if food[key].is_null() { Cow::Borrowed("未知") } else { display(&food[key]) };
                    append(&row, &node("td", &value, ""));
                }
                append(&body, &row);
            }
            append(&table, &body);
            append(&scroll, &table);
            append(&content, &scroll);
            append(&card, &summary);
            append(&card, &content);
            append(&list, &card);
        }
    }
    text("result-summary", &format!("检查 {} 个组合，{} 个符合条件，展示 {} 个。", display(&result["search"]["examined"]), display(&result["search"]["feasible"]), plans.len()));
    text("ranking", &format!("排序：{}。不是健康评分。", display(&result["ranking"])));
    hidden("ranking", result["status"] != "ok");
    hidden("download", false);
    hidden("export-note", false);
}

fn import_catalog(shared: &Shared) {
    let file_input = input("catalog-file");
    let Some(file) = file_input.files().and_then(|files| files.item(0)) else { return };
    if shared.borrow().online_busy {
        file_input.set_value("");
        return;
    }
    let generation = {
        let mut state = shared.borrow_mut();
        state.importing += 1;
        invalidate(&mut state);
        state.importing
    };
    let shared = shared.clone();
    spawn_local(async move {
        let result = async {
            if file.size() > (2 * 1024 * 1024) as f64 {
                return Err("目录文件不能超过 2 MiB。".to_owned());
            }
            let raw = JsFuture::from(file.text()).await.map_err(|_| "无法读取目录文件。".to_owned())?
                .as_string().ok_or_else(|| "无法读取目录文件。".to_owned())?;
            api(prepare_request("/api/catalog", Some(&raw))).await
        }.await;
        file_input.set_value("");
        let mut state = shared.borrow_mut();
        if generation != state.importing {
            return;
        }
        match result {
            Ok(catalog) => {
                apply_catalog(&mut state, Some(Rc::new(catalog)), false, false);
                message("目录已导入本次页面；请重新选择保留餐品。未写入文件或上传外部服务。", false, "message");
            }
            Err(error) => message(&format!("导入失败：{error} 原目录未改变。"), true, "message"),
        }
    });
}

#[derive(Serialize)]
struct PlanRequest<'a> {
    catalog: &'a Value,
    constraints: Value,
}

fn plan(shared: &Shared) {
    let request;
    let revision;
    {
        let mut state = shared.borrow_mut();
        if state.catalog.is_none() || state.busy || state.online_busy {
            return;
        }
        if state.selected.is_empty() {
            message("请先选择至少一种要保留的餐品。", true, "message");
            let _ = input("search").focus();
            return;
        }
        let exclude: HtmlTextAreaElement = element("exclude").unchecked_into();
        let max_items: HtmlSelectElement = element("max-items").unchecked_into();
        let top: HtmlSelectElement = element("top").unchecked_into();
        let max_sodium = input("max-sodium").value();
        let constraints = json!({
            "keep": state.selected,
            "exclude": exclude.value().lines().map(str::trim).filter(|term| !term.is_empty()).collect::<Vec<_>>(),
            "min_kcal": input("min-kcal").value(),
            "max_kcal": input("max-kcal").value(),
            "min_protein": input("min-protein").value(),
            "max_sodium": if max_sodium.is_empty() { None } else { Some(max_sodium) },
            "max_items": max_items.value().parse::<u32>().expect("max-items options"),
            "top": top.value().parse::<u32>().expect("top options"),
        });
        let body = serde_json::to_string(&PlanRequest {
            catalog: state.catalog.as_ref().expect("catalog checked"),
            constraints,
        }).expect("JSON request");
        request = prepare_request("/api/plan", Some(&body));
        dismiss_message();
        invalidate(&mut state);
        revision = state.revision;
        state.busy = true;
        update_controls(&state);
    }
    text("generate", "正在计算…");
    element("results-section").set_attribute("aria-busy", "true").expect("results busy state");
    text("result-summary", "正在检查满足全部条件的组合…");
    let shared = shared.clone();
    spawn_local(async move {
        let result = api(request).await;
        let mut state = shared.borrow_mut();
        if revision == state.revision {
            match result {
                Ok(result) => {
                    render_result(&result);
                    state.result = Some(result);
                }
                Err(error) => {
                    message(&error, true, "message");
                    text("result-summary", "未生成方案，请检查上方错误并调整输入。");
                }
            }
        }
        state.busy = false;
        update_controls(&state);
        text("generate", "生成搭配 ↗");
        element("results-section").set_attribute("aria-busy", "false").expect("results busy state");
    });
}

fn download(state: &State) -> Result<(), JsValue> {
    let Some(result) = &state.result else { return Ok(()) };
    let raw = serde_json::to_string_pretty(result).expect("JSON result") + "\n";
    let parts = Array::new();
    parts.push(&JsValue::from_str(&raw));
    let options = BlobPropertyBag::new();
    options.set_type("application/json");
    let blob = Blob::new_with_str_sequence_and_options(&parts, &options)?;
    let url = Url::create_object_url_with_blob(&blob)?;
    let link: HtmlAnchorElement = node("a", "", "").unchecked_into();
    link.set_href(&url);
    link.set_download("maimai-plan.json");
    document().body().expect("document body").append_child(&link)?;
    link.click();
    link.remove();
    let cleanup_url = url.clone();
    let cleanup = Closure::once_into_js(move || {
        let _ = Url::revoke_object_url(&cleanup_url);
    });
    if let Err(error) = window().set_timeout_with_callback_and_timeout_and_arguments_0(cleanup.unchecked_ref(), 1000) {
        Url::revoke_object_url(&url)?;
        return Err(error);
    }
    Ok(())
}

fn listen(target: &EventTarget, kind: &str, handler: impl FnMut(Event) + 'static) -> Result<(), JsValue> {
    let handler = Closure::wrap(Box::new(handler) as Box<dyn FnMut(Event)>);
    target.add_event_listener_with_callback(kind, handler.as_ref().unchecked_ref())?;
    // Only the fixed page-lifetime listeners are retained; rows use delegation below.
    handler.forget();
    Ok(())
}

fn on(id: &str, kind: &str, handler: impl FnMut(Event) + 'static) -> Result<(), JsValue> {
    listen(element(id).as_ref(), kind, handler)
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    let shared = Shared::default();
    let state = shared.clone();
    on("online-form", "submit", move |event| {
        event.prevent_default();
        online_request(&state, false);
    })?;
    let state = shared.clone();
    on("inspect-tool", "click", move |_| online_request(&state, true))?;
    listen(window().as_ref(), "pagehide", move |_| input("mcp-token").set_value(""))?;
    let state = shared.clone();
    on("search", "input", move |_| render_foods(&state.borrow()))?;
    let state = shared.clone();
    on("food-list", "change", move |event| {
        if let Some(checkbox) = event.target().and_then(|target| target.dyn_into::<HtmlInputElement>().ok()) {
            select_food(&mut state.borrow_mut(), checkbox.value(), checkbox.checked());
        }
    })?;
    let state = shared.clone();
    on("selected-list", "click", move |event| {
        let button = event.target().and_then(|target| target.dyn_into::<Element>().ok())
            .and_then(|target| target.closest("button[data-food]").ok().flatten());
        if let Some(name) = button.and_then(|button| button.get_attribute("data-food")) {
            select_food(&mut state.borrow_mut(), name, false);
        }
    })?;
    let state = shared.clone();
    on("clear", "click", move |_| {
        let mut state = state.borrow_mut();
        if state.online_busy { return; }
        state.selected.clear();
        dismiss_message();
        invalidate(&mut state);
        render_selected(&state);
    })?;
    let state = shared.clone();
    on("restore", "click", move |_| {
        let mut state = state.borrow_mut();
        if state.online_busy { return; }
        state.importing += 1;
        let startup = state.startup.clone();
        apply_catalog(&mut state, startup, true, false);
        dismiss_message();
    })?;
    let state = shared.clone();
    on("plan-form", "input", move |_| {
        dismiss_message();
        invalidate(&mut state.borrow_mut());
    })?;
    let state = shared.clone();
    on("catalog-file", "change", move |_| import_catalog(&state))?;
    let state = shared.clone();
    on("plan-form", "submit", move |event| {
        event.prevent_default();
        plan(&state);
    })?;
    let state = shared.clone();
    on("download", "click", move |_| {
        if download(&state.borrow()).is_err() {
            message("无法导出 JSON，请检查浏览器下载权限。", true, "message");
        }
    })?;
    apply_catalog(&mut shared.borrow_mut(), None, false, false);
    let generation = shared.borrow().importing;
    spawn_local(async move {
        let result = api(prepare_request("/api/catalog", None)).await;
        let mut state = shared.borrow_mut();
        match result {
            Ok(catalog) => {
                state.startup = if catalog.is_null() { None } else { Some(Rc::new(catalog)) };
                if generation == state.importing {
                    let startup = state.startup.clone();
                    apply_catalog(&mut state, startup, true, false);
                    text("result-summary", if state.catalog.is_some() {
                        "选好餐品、设定条件，然后生成搭配。"
                    } else {
                        "请先获取官方营养数据或导入目录。"
                    });
                } else {
                    text("restore", if state.startup.is_some() { "恢复启动目录" } else { "清除当前目录" });
                    update_controls(&state);
                }
            }
            Err(error) if generation == state.importing => {
                message(&error, true, "message");
                text("source-kind", "目录加载失败");
                element("food-list").set_attribute("aria-busy", "false").expect("food busy state");
                update_controls(&state);
            }
            Err(_) => {}
        }
    });
    Ok(())
}
