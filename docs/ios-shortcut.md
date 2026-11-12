# Adding GIFs from an iPhone with a Shortcut

iOS Safari cannot receive shares the way Android's installed app does, so on an iPhone or iPad the route
into Unigif from other apps is an **iOS Shortcut** in the share sheet. It posts the shared GIF (or link)
to the quick-add endpoint with an API key, and the GIF lands in Unsorted like any other.

> These steps follow the Shortcuts app as documented; they have not been tried on a device yet (that check is
> one of the owner's steps in `PLAN.md`, section 13.2). If a step is named differently on your iOS version,
> the shape stays the same: receive input, POST it, show the result.

## 1. Make an Uploader key

In Unigif, open **Settings → API keys**, create a key named "iPhone Shortcut" with the **Uploader** preset,
and copy it. An Uploader key can add GIFs but not browse or change the library, so a lost phone cannot leak it.
The key is shown only once; it looks like `ugf_…`.

## 2. Build the Shortcut

In the Shortcuts app, tap **+** and name the shortcut "Add to Unigif". Then:

1. Tap the **ⓘ** (details) button, turn on **Show in Share Sheet**, and set the accepted types to
   **Images**, **Media**, **Files** and **URLs**. The first action becomes
   *Receive Images, Media, Files and URLs input from Share Sheet*. Set *If there's no input* to
   **Ask For Files**, so the shortcut also works when started on its own.

2. Add **If**: *Shortcut Input* **has any value** … is a **URL**. (In recent iOS versions: *If Shortcut Input
   is URL*.) This splits links from files.

3. Inside the **If** branch (a link, for example from Safari or GIPHY), add **Get Contents of URL**:
   - URL: `https://gif.example.com/api/v1/gifs?wait=0`
   - Method: **POST**
   - Headers: `Authorization` = `Bearer ugf_…` (your key)
   - Request Body: **JSON**, with one field `url` (Text) = *Shortcut Input*

   The server fetches the link itself, with the same protections as any quick add by URL.

4. In the **Otherwise** branch (a file, for example from Photos or Files), add another **Get Contents of URL**:
   - URL: `https://gif.example.com/api/v1/gifs?source=share&wait=0`
   - Method: **POST**
   - Headers: `Authorization` = `Bearer ugf_…`
   - Request Body: **File** = *Shortcut Input*

   The body is sent as raw bytes; the server works out the type from the content.

5. After **End If**, add **Get Dictionary Value**: *Get* **Value** for `status` in *Contents of URL*, then
   **Show Notification** with the text "Added to Unigif (*Dictionary Value*)". A new GIF says `processing` (it
   is being converted) or `ready`; one you already had comes back with `"duplicate": true`.

Replace `gif.example.com` with your own `publicUrl`.

## Hashtags at once (optional)

To tag while sharing, add **Ask for Input** (Text, prompt "Hashtags") before the **If**, and append
`&tags=` followed by the *Provided Input* to both URLs (for the JSON request, a `tags` field works too).
Hashtags are separated by spaces or commas; `#` is optional. Without hashtags the GIF waits in Unsorted.

## When it does not work

- **401 Unauthorized**: the header is not exactly `Bearer ` plus the key, or the key was revoked. Keys in the
  URL are refused on purpose.
- **403 / "scope required"**: the key is not allowed to upload; use the Uploader (or Picker) preset.
- **415 "unsupported file type"**: the app shared a still image (Photos sometimes converts a GIF to JPEG or
  HEIC). Share the GIF from the Files app, or save it to Files first. Unigif accepts GIF, WebP, PNG, MP4,
  WebM and MKV.
- **413**: the file is larger than `upload.maxSize` (50 MiB by default).
- **429**: the key's rate limit; wait a minute.
