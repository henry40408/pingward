package views

// goldenDigests holds the sha256 of each page the Rust v0.12.0 server rendered
// (ghcr.io/henry40408/pingward:latest, revision ca5682f, over HTTP), after
// normalise. The server's state was built through HTTP only: users admin and
// bob (both admins; bob was promoted) and carol (not an admin, no projects),
// 3 projects, 6 checks (nightly-backup with period, max runtime and nag;
// weekly-report on a cron in Asia/Taipei; heartbeat; flaky with a 2s period;
// slow-job, which overran; bob-check), 7 channels (one per notifier kind on
// admin's project, plus bob-hook), 2 API keys (ci, script), 14 pings, 47
// notifications (down, up, reminder), 10 audit rows, 8 settings and 3
// sessions. Admin pages came from an elevated admin session and confirmation
// pages are the POST response without ?confirmed, with a valid CSRF token.
// Three names are not whole pages:
//
//   - admin is the page with its Environment card replaced by "ENV", because
//     that table lists the capture server's own environment.
//   - login-shell is the login page with its <main> element replaced by
//     <main class="wrap">BODY</main>, which pins the layout around a page.
//   - dashboard-nav is the <header> element of the dashboard page, which pins
//     the navigation bar.
//
// Do not recompute a digest to make a test pass: it is the Rust server's page.
var goldenDigests = map[string]string{
	"account":                        "2b23215705e3539797c47e4390673a07b3f93253d9106fc2a9ac1984a00ffd55",
	"account-key-created":            "b41069d93a063443a96bc8231d4d335e577b34300b289c33af9cae560f88684f",
	"admin":                          "d92e9f23ff56cc425a1589694e2d50f52dea43e53dadba12b015adb53c62a7ba",
	"admin-audit-filter":             "ee6015f791acdb329391722e140751f558ce12841d20ede15ccacc9d7e8eb413",
	"admin-channel-edit":             "649b377502a3d282ae9ef874a2422a7183a882b42705db739ccb26cb8a55c04d",
	"admin-channel-new":              "2d802348fb6e35729427e9160070b0913873dcda574022dc71990b8b632a4df3",
	"admin-unlock":                   "684ccbb7d87a25efac3a3c741273f30b6757d919eb05d53757df62e184bcb4ce",
	"channel-edit-1":                 "a52114a3e7865f5b9b6842afabd1f5c5dc04e01534cba57c4eebe2e754aa8d99",
	"channel-edit-2":                 "df39a6ed40d77fce856a9f632195d2da1b3b7594eae4f47609a984dca34eb6d4",
	"channel-edit-3":                 "2a4704a99e645c74929ae51c02e7122a9da8af126aee05e5a33c7a1883dbe7e7",
	"channel-edit-4":                 "a2baf619472a71aff491676dbd43d9e4777ca401d7ad80adb83329eb3da1e16c",
	"channel-edit-5":                 "69c716b50d347924b203a882e3de6445cb712406ebcca28d683bef35564babec",
	"channel-edit-6":                 "9ac5ccb769daf69a6430777e42083bdb28f0ff93c7aaff2b5d00665da175d186",
	"channel-new":                    "f853195b0f879fe34b1dae1abd56a6253ee0006d5cab5055f76686cd1562a1e5",
	"check-edit":                     "8358a844c8fe149178e52342595050312bea8835a4d4d40e77cb9a51723fac80",
	"check-new-state":                "64d6bd8aa0917e19920997850b69c0e5103a4944d7290a1d4bf36c91d0fc382e",
	"check-notifications-empty":      "09b960b97fdd48a2eeb0443b75173fabeba9c7ac00155c6e91520d1b2f0657aa",
	"confirm-admin-channel-delete":   "0efc06d50ec38e50654ebf2a7a51a0c2c8c418eca5a21ff59002bb97518151ff",
	"confirm-admin-check-delete":     "8bc690d631021c4e32e287804e3d46cd67c4b5436b284ae474c7957afe223580",
	"confirm-admin-check-regenerate": "0aacdcb78280f11f977a80f678b1ae1514565732232bd81343b5102b4bc642ec",
	"confirm-admin-project-delete":   "17e593ed12fea853a61d00c487e7f2daf64aca1e2ac01942d977cc470791db68",
	"confirm-api-key-delete":         "b0b7458e8474abb4ce8fba72a2cb7502e28df4825ccc8e97d5be5112f79a1bc9",
	"confirm-channel-delete":         "a48c3e1b3343c1f4eae4d13df370971d81a3a3f7ab6c0b181ec7fbf0e33f5299",
	"confirm-check-delete":           "d5c329973c8df163bf4b091863bb730a2eb8480eec585202058775382c023315",
	"confirm-check-regenerate":       "0d3247802c252eb2f87ad80fd2649a62df5618d3f51981e23f54ce194aeff5cb",
	"confirm-project-delete":         "d13ec3dafbfc5c44d4e4d8073ea882c428fd9a3ef37682a2f8ddd8cd2960200f",
	"confirm-sessions-revoke-others": "1c83c3ff79e9598e1ac47dbfa7d71204440e8e2cb59fe7b797e20614e53b6f51",
	"confirm-users-delete":           "b378186845da7bce447271f9a93210fd41dc03432d4531fc3dfe967dd6bf8ee9",
	"confirm-users-disable":          "262cb214723bff104045a425f400cd7de36d0d9c2d5b2550e380607d01d668bd",
	"confirm-users-toggle-admin":     "bdf2e37acfc6a26b276ba7b6e6d535f7e9db169251c2ff14a825afc26ae899c2",
	"dashboard-down":                 "ca111146feb7f68d9bfce1cd22bcad855d0c0e22a6984cdb542d34f5aa2ee580",
	"dashboard-empty":                "3e224d07df975089d31541c3719ef4f3886800d15c49f0d2d158e6197d94997c",
	"dashboard-nav":                  "5fce07cf0ff8d0c4dc5619016cf22c4275361da71504282a3da3572e80946c97",
	"login":                          "56773c2287ccbea7a61a5e90efb852d174b6c1523c7db629e4305557cfb5dfcf",
	"login-shell":                    "ca7d5bcdf4c369575ae1dbc2518057f14d50f9083a87ddc10a0195344296616b",
	"project":                        "4e7a5a6d34b7503fcfa904157f78de229af2149f3b5bdaaa237f1c6e460dece6",
	"project-edit":                   "f0dde114accd33b4d7a191926523c8084fc58dd3b1ddc5c062a8abc292de8c40",
	"projects-new":                   "fd9f21ffcf846408d0f24d61ebc722cdb65407c2e3528ba7603a8e8e7265b05e",
	"setup":                          "83d3a3e11aef2146c6f90473347c3f114bb322a028c3480c8a79c35ef7bfae26",
}

// confirmCase is the input of one confirmation page; the Rust page showed
// exactly these strings.
type confirmCase struct {
	name                                   string
	isAdmin                                bool
	title, message, action, button, cancel string
}

var confirmCases = []confirmCase{
	{name: "confirm-admin-channel-delete", isAdmin: true, title: "Delete this channel?", message: "Checks bound to it stop notifying through it. This cannot be undone.", action: "/admin/channels/8/delete?confirmed=1", button: "Delete channel", cancel: "/admin/projects/4"},
	{name: "confirm-admin-check-delete", isAdmin: true, title: "Delete this check?", message: "The check and its whole ping history go away, and its ping URL stops answering — a job still calling it will start getting 404s. This cannot be undone.", action: "/admin/checks/8/delete?confirmed=1", button: "Delete check", cancel: "/admin/checks/8"},
	{name: "confirm-admin-check-regenerate", isAdmin: true, title: "Regenerate this check's ping URL?", message: "The current URL stops working immediately. Every job that pings this check must be updated to the new URL, or the check will go down when its next ping never arrives.", action: "/admin/checks/8/regenerate?confirmed=1", button: "Regenerate URL", cancel: "/admin/checks/8"},
	{name: "confirm-admin-project-delete", isAdmin: true, title: "Delete this project?", message: "Deleting a project deletes everything inside it: every check it holds, their ping history, and the notification channels configured on it. Any job still pinging one of those checks will start getting 404s. This cannot be undone.", action: "/admin/projects/4/delete?confirmed=1", button: "Delete project", cancel: "/admin/projects/4"},
	{name: "confirm-api-key-delete", isAdmin: true, title: "Revoke this API key?", message: "Anything using this key stops being able to reach the API immediately. The key cannot be recovered — a replacement is a new key with a new token.", action: "/account/api-keys/1/delete?confirmed=1", button: "Revoke key", cancel: "/account"},
	{name: "confirm-channel-delete", isAdmin: true, title: "Delete this channel?", message: "Checks bound to it stop notifying through it. This cannot be undone.", action: "/channels/1/delete?confirmed=1", button: "Delete channel", cancel: "/projects/1"},
	{name: "confirm-check-delete", isAdmin: true, title: "Delete this check?", message: "The check and its whole ping history go away, and its ping URL stops answering — a job still calling it will start getting 404s. This cannot be undone.", action: "/checks/1/delete?confirmed=1", button: "Delete check", cancel: "/checks/1"},
	{name: "confirm-check-regenerate", isAdmin: true, title: "Regenerate this check's ping URL?", message: "The current URL stops working immediately. Every job that pings this check must be updated to the new URL, or the check will go down when its next ping never arrives.", action: "/checks/1/regenerate?confirmed=1", button: "Regenerate URL", cancel: "/checks/1"},
	{name: "confirm-project-delete", isAdmin: true, title: "Delete this project?", message: "Deleting a project deletes everything inside it: every check it holds, their ping history, and the notification channels configured on it. Any job still pinging one of those checks will start getting 404s. This cannot be undone.", action: "/projects/1/delete?confirmed=1", button: "Delete project", cancel: "/projects/1"},
	{name: "confirm-sessions-revoke-others", isAdmin: true, title: "Revoke every other session?", message: "Every other signed-in browser is signed out at once. Only this one stays. API keys are unaffected — they are separate credentials, revoked from the API keys card below.", action: "/account/sessions/revoke-others?confirmed=1", button: "Revoke the others", cancel: "/account"},
	{name: "confirm-users-delete", isAdmin: true, title: "Delete this user?", message: "The account goes away along with everything it owns: its projects, their checks, and the whole ping history behind them. This cannot be undone. To cut off access while keeping the data, disable the account instead.", action: "/admin/users/2/delete?confirmed=1", button: "Delete user", cancel: "/admin"},
	{name: "confirm-users-disable", isAdmin: true, title: "Disable this user?", message: "They cannot sign in again until the account is re-enabled. Nothing they own is deleted, and their checks keep running and alerting.", action: "/admin/users/2/disabled?confirmed=1", button: "Disable user", cancel: "/admin"},
	{name: "confirm-users-toggle-admin", isAdmin: true, title: "Revoke this user's admin rights?", message: "They keep their account and their own projects, but lose access to /admin and to every other user's data.", action: "/admin/users/2/admin?confirmed=1", button: "Revoke admin rights", cancel: "/admin"},
}
