mod sidebar;

use leptos::*;
use leptos_router::*;
use sidebar::NavSidebar;

#[component]
pub fn Dashboard() -> impl IntoView {
    view! {
        <div class="backdrop-blue-green">
            <div class="p-16 flex flex-col sm:flex-row justify-items-stretch content-stretch sm:space-x-8 sm:space-y-0 space-y-8">
                <div class="basis-1/3 rounded-xl bg-black/50 p-4">
                    <NavSidebar/>
                </div>
                <div class="basis-2/3 rounded-xl bg-black/50 p-4">
                    <Outlet/>
                </div>
            </div>
        </div>
    }
}
