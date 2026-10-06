<!-- source: express/lib/view.js; section: 1320e03018a51821; licence: MIT -->
### User
In Express I call `res.render('index')` with `app.set('view engine', 'pug')`. How does Express find the file and the template engine?

### Assistant
`res.render` creates a `View` (in `lib/view.js`), and its constructor does three things.

1. **Works out the extension.** If the name has one (`index.pug`), that is used. If not, the default engine supplies it: `'pug'` becomes `.pug` (a leading dot is added when missing), and the file name becomes `index.pug`. With no extension and no default engine, it throws `No default engine was specified and no extension was provided.`
2. **Loads the engine once.** Engines are cached by extension in the app's `engines` object. On first use of `.pug`, Express calls `require('pug').__express` and checks that it is a function; if not, it throws `Module "pug" does not provide a view engine.` Later renders reuse the cached function.
3. **Finds the file.** `views` may be one directory or an array. Each root is tried in order with `path.resolve(root, name)`, and the first root where the file resolves wins.

The same rules, in a small function you can test:

```javascript
const assert = require("node:assert");

function viewFile(name, defaultEngine) {
  const dot = name.lastIndexOf(".");
  if (dot > 0) {
    return { file: name, ext: name.slice(dot) };
  }
  if (!defaultEngine) {
    throw new Error("No default engine was specified and no extension was provided.");
  }
  const ext = defaultEngine.startsWith(".") ? defaultEngine : `.${defaultEngine}`;
  return { file: name + ext, ext };
}

assert.deepStrictEqual(viewFile("index", "pug"), { file: "index.pug", ext: ".pug" });
assert.deepStrictEqual(viewFile("index", ".ejs"), { file: "index.ejs", ext: ".ejs" });
assert.deepStrictEqual(viewFile("mail.html", "pug"), { file: "mail.html", ext: ".html" });
assert.throws(() => viewFile("index"), /No default engine/);
```

The real code uses `path.extname`, which also handles names like `.hidden` and directories with dots; this sketch only shows the order of decisions.

### User
My templates are `.html` files but I render them with EJS. How do I tell Express?

### Assistant
Register the engine for that extension yourself, so Express never tries `require('html')`:

```javascript,ignore
const ejs = require("ejs");

app.engine("html", ejs.renderFile);
app.set("view engine", "html");
app.set("views", [path.join(__dirname, "views"), path.join(__dirname, "shared-views")]);
```

`app.engine` fills the same `engines` cache that the `View` constructor checks, so the lookup finds `.html` already registered and skips the `require`. `renderFile` has the `(path, options, callback)` signature Express expects. Passing an array to `views` makes Express look in each directory in order, which is handy for shared layouts.
