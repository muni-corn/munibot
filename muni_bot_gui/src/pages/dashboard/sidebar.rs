use leptos::*;
use leptos_router::*;

#[component]
pub fn NavSidebar() -> impl IntoView {
    let nav_items: Vec<(&str, &str)> = vec![("One", "/dashboard/one"), ("Two", "/dashboard/two")];
    let route = use_router().pathname();

    let nav_items_with_classes = move || {
        nav_items
            .iter()
            .map(|(name, href)| {
                log::info!("route: {}, href: {}", route(), href);
                (*name, *href)
            })
            .collect::<Vec<(&str, &str)>>()
    };

    view! {
        <nav class="sidebar-nav">
            <ul>
                <For each=nav_items_with_classes key=|(_, href)| *href let:item>
                    <A href=item.1>
                        <li class=if route().starts_with(item.0) {
                            "sidebar-nav-item active"
                        } else {
                            "sidebar-nav-item "
                        }>{item.0}</li>
                    </A>
                </For>
            </ul>
        </nav>
    }
}
