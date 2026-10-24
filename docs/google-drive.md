# Google Drive

File Browser can show your Google Drive next to your local folders. Drive
support is **experimental**: you can browse folders, see thumbnails and
properties, open files, create folders, rename, cut, copy and paste (inside
Drive and to or from your computer) and move items to Drive's Trash. Nothing
is ever deleted permanently.

File Browser doesn't come with its own Google sign-in. Instead, you create a
free **OAuth client** in your own Google Cloud project and give its file to
File Browser once. This takes about 10 minutes and you only do it once. It
means:

- Your Drive access goes through your own Google project. No one else's app
  is involved.
- There's no shared user limit and nothing for the File Browser developers to
  get approved by Google.

> File Browser is an independent hobby project. It is not made, endorsed or
> verified by Google.

## What you need

- A Google account. A personal Gmail account works.
- A web browser to use the Google Cloud Console.
- No payment details: everything here is free.

## Part 1: Create your OAuth client

Google renames these pages from time to time. If a button has a slightly
different name, look for the closest match.

### 1. Create a project

1. Open <https://console.cloud.google.com/projectcreate>.
2. Name it, for example **File Browser**, and click **Create**.
3. Make sure the new project is selected in the project picker at the top of
   the page for the steps below.

### 2. Turn on the Google Drive API

1. Open <https://console.cloud.google.com/apis/library/drive.googleapis.com>.
2. Click **Enable**.

### 3. Set up the consent screen

1. Open <https://console.cloud.google.com/auth/branding>. If you see
   **Get started**, click it.
2. **App name:** anything you like, such as *My File Browser*. Don't use
   "Google" in the name; Google rejects it.
3. **User support email:** your email address.
4. **Audience:** choose **External**.
5. **Contact information:** your email address.
6. Accept the policy and click **Create**.

Leave the logo empty. Adding a logo makes Google require a review before the
app can be used.

### 4. Add yourself as a test user

1. Open <https://console.cloud.google.com/auth/audience>.
2. Under **Test users**, click **Add users** and enter the Google account whose
   Drive you want to browse.
3. Click **Save**.

### 5. Create the client and download its file

1. Open <https://console.cloud.google.com/auth/clients>.
2. Click **Create client**.
3. **Application type:** choose **Desktop app**. Other types such as "Web
   application" don't work with File Browser.
4. Name it, for example *File Browser*, and click **Create**.
5. In the window that appears, click **Download JSON** and keep the file. Its
   name looks like `client_secret_….apps.googleusercontent.com.json`.

**Download the file right away.** Google may not show the secret again
later. If you lost the file, open the client, add a new secret, and download
the JSON again.

## Part 2: Connect File Browser

1. In File Browser's sidebar, click **Google Drive**.
2. The **Add Your Google Client** dialog opens. Click **Choose File…** and pick
   the JSON file you downloaded. You can also open the file in a text editor,
   copy all of it, paste it into the box, and click **Add Client**.
3. In the **Connect to Google Drive** dialog, click **Open Browser**.
4. Sign in to Google in your browser.
5. Google says **"Google hasn't verified this app"**. This is expected: the
   app it means is the client you just created in your own project. Click
   **Advanced**, then **Go to *your app name* (unsafe)**.
6. Allow File Browser to **see, edit, create and delete all of your Google
   Drive files**.
7. When the browser says you can close the window, go back to File Browser.
   Your Drive opens.

## Signing in every 7 days

While your Google project is in **Testing** mode, Google ends the session
after 7 days, and File Browser asks you to sign in again. To stop this:

1. Open <https://console.cloud.google.com/auth/audience>.
2. Click **Publish app** and confirm. You don't need Google's verification for
   your own use.
3. In File Browser, right-click **Google Drive** in the sidebar, choose
   **Disconnect…**, then connect again. The new session doesn't expire.

The "unverified app" warning still appears when you sign in. That's normal.

## Privacy and where things are stored

- File Browser talks only to Google, and only from your computer. Nothing is
  sent anywhere else.
- It changes files in your Drive only when you ask it to: New Folder,
  Rename, Paste, drag and drop and Move to Trash. It never deletes anything
  permanently: Move to Trash uses Drive's Trash, which keeps items for 30
  days. It doesn't share anything.
- Your sign-in token is kept in the system keychain: Keychain on macOS, the
  Secret Service (GNOME Keyring or KWallet) on Linux. If there's no keychain,
  it falls back to a file only your user can read.
- Your OAuth client is saved in `google-client.json`, a file only your user can
  read, in File Browser's settings folder:
  - macOS: `~/Library/Application Support/io.github.ahmadsayed.filebrowser/`
  - Linux: `~/.config/io.github.ahmadsayed.filebrowser/`
- Files you open from Drive are downloaded as read-only copies to File
  Browser's cache. Changes to those copies are **not** uploaded to Drive.
- **Disconnect** removes the token from your computer and asks Google to revoke
  it. You can also remove access at <https://myaccount.google.com/connections>.

## Using a different client

Disconnect Google Drive first: right-click it in the sidebar and choose
**Disconnect…**. Then click **Google Drive** and choose **Change Client…** in
the Connect dialog.

## Troubleshooting

| What you see | What to do |
| --- | --- |
| **Error 403: access_denied** / "has not completed the Google verification process" | Your account isn't a test user. Add it in [step 4](#4-add-yourself-as-a-test-user), or publish the app as described in [Signing in every 7 days](#signing-in-every-7-days). |
| "This is a web application client" | Create a new client with the type **Desktop app** ([step 5](#5-create-the-client-and-download-its-file)). |
| "This is not an OAuth client file" | Choose the JSON file you downloaded from the client's page, not another file from Google Cloud. |
| "Google Drive API has not been used in project … or it is disabled" | Enable the API ([step 2](#2-turn-on-the-google-drive-api)), wait a few minutes and try again. |
| **invalid_client** or "The OAuth client was not found" | The client was deleted, or its secret was reset. Download the JSON again, then use **Change Client…**. |
| "Disconnect Google Drive first." | You tried to change the client while connected. Disconnect, then change it. |
| You're asked to sign in again every week | Your project is in Testing mode; see [Signing in every 7 days](#signing-in-every-7-days). |
| "Google Drive is connected read-only" | Your connection was made before File Browser could change files. Disconnect Google Drive and connect again. |
| "Sign-in timed out" | The browser didn't come back within 3 minutes. Click **Open Browser** again. |
