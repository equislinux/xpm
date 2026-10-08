<h1 align="center" id="x-package-manager">X Package Manager</h1>

<p>Modern, high-performance package manager written in pure Rust for X</p>

<p align="center">
    <img alt="License" src="https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg">
    <img alt="Rust" src="https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white">
    <img alt="Version" src="https://img.shields.io/github/v/release/equislinux/xpm?include_prereleases&label=version">
    <img alt="CI" src="https://img.shields.io/github/actions/workflow/status/equislinux/xpm/ci.yml?branch=main&label=CI&logo=github">
    <img alt="Status" src="https://img.shields.io/badge/Status-Active%20Development-2ea44f">
</p>

<h2 align="center" id="menu">Menu</h2>

<ul>
    <li><a href="#overview">Overview</a></li>
    <li><a href="#key-features">Key Features</a></li>
    <li><a href="#installation">Installation</a></li>
    <li><a href="#quick-install-published-package">Quick Install (Published Package)</a></li>
    <li><a href="#build-from-source">Build From Source</a></li>
    <li><a href="#usage">Usage</a></li>
    <li><a href="#pacman-style-aliases">Pacman-Style Aliases</a></li>
    <li><a href="#global-flags">Global Flags</a></li>
    <li><a href="#configuration">Configuration</a></li>
    <li><a href="#signed-repository-bootstrap">Signed Repository Bootstrap</a></li>
    <li><a href="#key-bootstrap-checklist-xpm-native-repository">Key Bootstrap Checklist (xpm Native Repository)</a></li>
    <li><a href="#signature-troubleshooting">Signature Troubleshooting</a></li>
    <li><a href="#repository-management">Repository Management</a></li>
    <li><a href="#project-structure">Project Structure</a></li>
    <li><a href="#technical-architecture">Technical Architecture</a></li>
    <li><a href="#dependency-resolution">Dependency Resolution</a></li>
    <li><a href="#package-format">Package Format</a></li>
    <li><a href="#security">Security</a></li>
    <li><a href="#repository-hosting">Repository Hosting</a></li>
    <li><a href="#relationship-with-xpkg">Relationship with xpkg</a></li>
    <li><a href="#roadmap">Roadmap</a></li>
    <li><a href="#license">License</a></li>
    <li><a href="#command-cheatsheet">Command Cheatsheet</a></li>
    <li><a href="#x">X</a></li>
</ul>

<h2 align="center" id="overview">Overview</h2>

<p><code>xpm</code> is a native Rust replacement for <code>pacman</code> and <code>libalpm</code>, designed for the X distribution. It uses the <code>.xp</code> package format (X Package) natively and maintains compatibility with Arch Linux <code>.pkg.tar.zst</code> packages. Packages are built with <a href="https://github.com/equislinux/xpkg"><code>xpkg</code></a>, the companion builder tool.</p>

<h3 align="center" id="key-features">Key Features</h3>

<ul>
    <li><strong>Pure Rust</strong> - zero C dependencies at any stage</li>
    <li><strong>Native .xp format</strong> - X Package format (tar.zst) with <code>.PKGINFO</code> / <code>.BUILDINFO</code> / <code>.MTREE</code> metadata</li>
    <li><strong>SAT-based dependency resolver</strong> - powered by <code>resolvo</code> with CDCL and watched-literal propagation</li>
    <li><strong>Arch compatible</strong> - reads <code>.pkg.tar.zst</code> packages and <code>alpm-repo-db</code> databases</li>
    <li><strong>Flexible repository management</strong> - predefined and temporary repos with <code>xpm repo add/remove/list</code></li>
    <li><strong>OpenPGP verification</strong> - detached signatures with Web of Trust model</li>
    <li><strong>TOML configuration</strong> - clean, human-readable config at <code>/etc/xpm.conf</code></li>
</ul>

<h2 align="center" id="installation">Installation</h2>

<h3 align="center" id="quick-install-published-package">Quick Install (Published Package)</h3>

<p>Install the latest published <code>xpm</code> build directly from the official repository:</p>

<pre><code class="language-bash">curl -fsSL https://raw.githubusercontent.com/equislinux/xpm/main/install.sh | bash
</code></pre>

<p>If <code>curl</code> is not available:</p>

<pre><code class="language-bash">wget -qO- https://raw.githubusercontent.com/equislinux/xpm/main/install.sh | bash
</code></pre>

<p>Optional environment variables for the installer:</p>

<ul>
    <li><code>XPM_PKG_URL</code>: override the package URL (for testing another build)</li>
    <li><code>INSTALL_PREFIX</code>: change install prefix (default: <code>/usr/local</code>)</li>
</ul>

<p>Example:</p>

<pre><code class="language-bash">INSTALL_PREFIX=/usr XPM_PKG_URL="https://equislinux.github.io/x-repo/x/x86_64/xpm-0.1.0-3-x86_64.xp" \
curl -fsSL https://raw.githubusercontent.com/equislinux/xpm/main/install.sh | bash
</code></pre>

<h3 align="center" id="build-from-source">Build From Source</h3>

<pre><code class="language-bash">git clone https://github.com/equislinux/xpm.git
cd xpm
cargo build --release
sudo cp target/release/xpm /usr/local/bin/
</code></pre>

<h2 align="center" id="usage">Usage</h2>

<pre><code class="language-bash"># Sync package databases
xpm sync

# Install packages
xpm install &lt;package&gt; [&lt;package&gt;...]

# Remove packages
xpm remove &lt;package&gt;

# System upgrade
xpm upgrade

# Search packages
xpm search &lt;query&gt;

# Query installed packages
xpm query

# Package info
xpm info &lt;package&gt;

# List files owned by a package
xpm files &lt;package&gt;

# Manage repositories
xpm repo list
xpm repo add &lt;name&gt; &lt;url&gt;
xpm repo remove &lt;name&gt;
</code></pre>

<h3 align="center" id="pacman-style-aliases">Pacman-Style Aliases</h3>

<table>
    <thead>
        <tr>
            <th>Alias</th>
            <th>Command</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td><code>xpm Sy</code></td>
            <td><code>xpm sync</code></td>
        </tr>
        <tr>
            <td><code>xpm S &lt;pkg&gt;</code></td>
            <td><code>xpm install &lt;pkg&gt;</code></td>
        </tr>
        <tr>
            <td><code>xpm R &lt;pkg&gt;</code></td>
            <td><code>xpm remove &lt;pkg&gt;</code></td>
        </tr>
        <tr>
            <td><code>xpm Su</code></td>
            <td><code>xpm upgrade</code></td>
        </tr>
        <tr>
            <td><code>xpm Q</code></td>
            <td><code>xpm query</code></td>
        </tr>
        <tr>
            <td><code>xpm Ss &lt;query&gt;</code></td>
            <td><code>xpm search &lt;query&gt;</code></td>
        </tr>
        <tr>
            <td><code>xpm Si &lt;pkg&gt;</code></td>
            <td><code>xpm info &lt;pkg&gt;</code></td>
        </tr>
        <tr>
            <td><code>xpm Ql &lt;pkg&gt;</code></td>
            <td><code>xpm files &lt;pkg&gt;</code></td>
        </tr>
    </tbody>
</table>

<h3 align="center" id="global-flags">Global Flags</h3>

<table>
    <thead>
        <tr>
            <th>Flag</th>
            <th>Description</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td><code>-c, --config &lt;PATH&gt;</code></td>
            <td>Custom configuration file</td>
        </tr>
        <tr>
            <td><code>-v, --verbose</code></td>
            <td>Increase verbosity (-v, -vv, -vvv)</td>
        </tr>
        <tr>
            <td><code>--no-confirm</code></td>
            <td>Skip confirmation prompts</td>
        </tr>
        <tr>
            <td><code>--root &lt;PATH&gt;</code></td>
            <td>Alternative installation root</td>
        </tr>
        <tr>
            <td><code>--dbpath &lt;PATH&gt;</code></td>
            <td>Alternative database directory</td>
        </tr>
        <tr>
            <td><code>--cachedir &lt;PATH&gt;</code></td>
            <td>Alternative cache directory</td>
        </tr>
        <tr>
            <td><code>--no-color</code></td>
            <td>Disable colored output</td>
        </tr>
    </tbody>
</table>

<h2 align="center" id="configuration">Configuration</h2>

<p>Configuration file: <code>/etc/xpm.conf</code> (TOML format).</p>

<p>See <a href="etc/xpm.conf.example">etc/xpm.conf.example</a> for all available options.</p>

<pre><code class="language-toml">[options]
root_dir = "/"
db_path = "/var/lib/xpm/"
cache_dir = "/var/cache/xpm/pkg/"
gpg_dir = "/etc/xpm/gnupg/"
sig_level = "optional"
parallel_downloads = 5

[[repo]]
name = "x"
server = [
        "https://equislinux.github.io/x-repo/x/$arch",
]
</code></pre>

<p>Optional additional repositories can be appended as extra <code>[[repo]]</code> blocks.</p>

<p>The keyring directory resolves at runtime through <code>effective_gpg_dir()</code>:
the configured <code>gpg_dir</code> wins when it exists, then the shared pacman keyring
(<code>/etc/pacman.d/gnupg</code>), then <code>/etc/xpm/gnupg</code>. A dedicated xpm keyring
(recommended for the signed <code>[x]</code> repository bootstrap below) is therefore just a
matter of pointing <code>gpg_dir</code> at it and creating it; no duplicate default is required.</p>

<h3 align="center" id="signed-repository-bootstrap">Signed Repository Bootstrap</h3>

<p>To enforce signature verification from the official repository, install the published trusted keyring and switch the repository to <code>required</code> mode:</p>

<pre><code class="language-bash"># System-wide keyring directory used by xpm (must match gpg_dir in config)
sudo install -d -m 755 /etc/xpm/gnupg

# Download repository public keyring
sudo curl -fsSL \
        https://equislinux.github.io/x-repo/x/x86_64/trustedkeys.gpg \
        -o /etc/xpm/gnupg/trustedkeys.gpg

# Optional: keep the ASCII-armored public key for auditing
sudo curl -fsSL \
        https://equislinux.github.io/x-repo/x/x86_64/signing.pub \
        -o /etc/xpm/gnupg/signing.pub
</code></pre>

<p>Then set:</p>

<pre><code class="language-toml">[options]
gpg_dir = "/etc/xpm/gnupg/"
sig_level = "required"
</code></pre>

<p>You can also override per repository:</p>

<pre><code class="language-toml">[[repo]]
name = "x"
server = ["https://equislinux.github.io/x-repo/x/$arch"]
sig_level = "required"
</code></pre>

<h3 align="center" id="key-bootstrap-checklist-xpm-native-repository">Key Bootstrap Checklist (xpm Native Repository)</h3>

<p>Use this checklist to avoid signature-related install failures when consuming the X native <code>.xp</code> repository:</p>

<pre><code class="language-bash"># 1) Ensure keyring directory exists
sudo install -d -m 755 /etc/xpm/gnupg

# 2) Import published keyring + public key
sudo curl -fsSL https://equislinux.github.io/x-repo/x/x86_64/trustedkeys.gpg \
        -o /etc/xpm/gnupg/trustedkeys.gpg
sudo curl -fsSL https://equislinux.github.io/x-repo/x/x86_64/signing.pub \
        -o /etc/xpm/gnupg/signing.pub

# 3) Confirm /etc/xpm.conf points to x endpoint and required signatures
sudo tee /etc/xpm.conf &gt;/dev/null &lt;&lt;'EOF'
[options]
root_dir = "/"
db_path = "/var/lib/xpm/"
cache_dir = "/var/cache/xpm/pkg/"
gpg_dir = "/etc/xpm/gnupg/"
sig_level = "required"
parallel_downloads = 5

[[repo]]
name = "x"
server = ["https://equislinux.github.io/x-repo/x/$arch"]
sig_level = "required"
EOF

# 4) Sync and install from signed .xp repository
sudo xpm sync
sudo xpm install xpkg
</code></pre>

<h3 align="center" id="signature-troubleshooting">Signature Troubleshooting</h3>

<ul>
    <li><code>signature required but could not be downloaded</code>:
        <ul>
            <li>Check that <code>.sig</code> exists for package/database in <code>x/x86_64</code> endpoint.</li>
        </ul>
    </li>
    <li><code>failed to load keyring</code> or <code>no certificates found in keyring</code>:
        <ul>
            <li>Confirm <code>gpg_dir</code> and <code>trustedkeys.gpg</code> path in <code>/etc/xpm.conf</code>.</li>
        </ul>
    </li>
    <li><code>signature is valid but key is unknown</code>:
        <ul>
            <li>Refresh <code>/etc/xpm/gnupg/trustedkeys.gpg</code> from published endpoint and re-sync.</li>
        </ul>
    </li>
    <li>Package not found:
        <ul>
            <li>Confirm xpm repository URL is <code>https://equislinux.github.io/x-repo/x/$arch</code> and not the pacman endpoint under <code>/repo/x86_64</code>.</li>
        </ul>
    </li>
</ul>

<h3 align="center" id="repository-management">Repository Management</h3>

<p>Predefined repositories are configured in <code>/etc/xpm.conf</code>. Temporary repositories can be added at runtime with <code>xpm repo add</code> and are stored in <code>/etc/xpm.d/</code>.</p>

<h2 align="center" id="project-structure">Project Structure</h2>

<pre><code class="language-text">xpm/
├── Cargo.toml                  # Workspace root
├── crates/
│   ├── xpm/                    # Binary crate (CLI frontend)
│   │   └── src/
│   │       ├── main.rs         # Entry point, logging, config, dispatch
│   │       └── cli.rs          # clap CLI definition
│   └── xpm-core/               # Library crate (core logic)
│       └── src/
│           ├── lib.rs           # Module root
│           ├── config.rs        # TOML configuration parser
│           ├── error.rs         # Error types
│           └── repo.rs          # Repository manager
├── etc/
│   └── xpm.conf.example        # Example configuration
└── ROADMAP.md                   # Development roadmap
</code></pre>

<h2 align="center" id="technical-architecture">Technical Architecture</h2>

<h3 align="center" id="dependency-resolution">Dependency Resolution</h3>

<p><code>xpm</code> uses a logic-based SAT solver (<code>resolvo</code>) that transforms package relationships into CNF boolean clauses:</p>

<table>
    <thead>
        <tr>
            <th>Requirement</th>
            <th>CNF Clause</th>
            <th>Meaning</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td>Dependency</td>
            <td><code>!foo OR bar</code></td>
            <td>If <code>foo</code> is installed, <code>bar</code> must be too</td>
        </tr>
        <tr>
            <td>Root requirement</td>
            <td><code>foo</code></td>
            <td>Target package is mandatory</td>
        </tr>
        <tr>
            <td>Conflict</td>
            <td><code>!bar_v1 OR !bar_v2</code></td>
            <td>Mutually exclusive versions</td>
        </tr>
    </tbody>
</table>

<p>The solver implements Unit Propagation with watched literals and Conflict-Driven Clause Learning (CDCL) for efficient backtracking.</p>

<h3 align="center" id="package-format">Package Format</h3>

<p>Packages use the ALPM <code>.pkg.tar.zst</code> format with Zstandard compression:</p>

<ul>
    <li><code>.PKGINFO</code> - package name, version, dependencies</li>
    <li><code>.BUILDINFO</code> - reproducible build environment</li>
    <li><code>.MTREE</code> - file integrity hashes</li>
    <li><code>.INSTALL</code> - optional pre/post install, upgrade, and remove scripts (executed by xpm during transactions)</li>
</ul>

<h3 align="center" id="security">Security</h3>

<ul>
    <li><strong>OpenPGP detached signatures</strong> (<code>.sig</code>) for packages and databases</li>
    <li><strong>Web of Trust</strong> model for key validation</li>
    <li><strong>Fakeroot</strong> build environment for safe package creation</li>
    <li><strong>Package linting</strong> framework for quality assurance</li>
</ul>

<h2 align="center" id="repository-hosting">Repository Hosting</h2>

<p>The default package repository is hosted on <strong>GitHub Pages</strong> at <code>equislinux.github.io/x-repo</code> (organization <code>equislinux</code>). <code>xpm</code> supports any HTTP-based static file server, making future migrations transparent.</p>

<h2 align="center" id="relationship-with-xpkg">Relationship with xpkg</h2>

<p><code>xpm</code> and <code>xpkg</code> are complementary tools in the X ecosystem. They share the same package format and metadata structures but are independent binaries.</p>

<table align="center">
    <thead>
        <tr>
            <th>Tool</th>
            <th>Role</th>
            <th>Analogy</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td><strong>xpm</strong></td>
            <td>Package manager — install, remove, upgrade, resolve deps</td>
            <td><code>pacman</code></td>
        </tr>
        <tr>
            <td><strong>xpkg</strong></td>
            <td>Package builder — compile, package, lint, manage repos</td>
            <td><code>makepkg</code> + <code>repo-add</code> + <code>namcap</code></td>
        </tr>
    </tbody>
</table>

<p><code>xpkg</code> produces <code>.xp</code> packages that <code>xpm</code> installs. During installation, <code>xpm</code> executes any <code>.INSTALL</code> scriptlets generated by <code>xpkg</code> (post_install, pre_upgrade, etc.) and verifies signatures created during the build process.</p>

<p>See the <a href="docs/INTEGRATION.md">Integration Guide</a> for detailed information about how the tools work together.</p>

<h2 align="center" id="roadmap">Roadmap</h2>

<p>See <a href="ROADMAP.md">ROADMAP.md</a> for the full development roadmap.</p>

<table>
    <thead>
        <tr>
            <th>Version</th>
            <th>Milestone</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td><code>v0.1.0</code></td>
            <td>Functional CLI with configuration</td>
        </tr>
        <tr>
            <td><code>v0.5.0</code></td>
            <td>Native engine (resolver + packages + repo db)</td>
        </tr>
        <tr>
            <td><code>v0.8.0</code></td>
            <td>Security and transaction management</td>
        </tr>
        <tr>
            <td><code>v1.0.0</code></td>
            <td>Benchmarked, tested, production-ready</td>
        </tr>
    </tbody>
</table>

<h2 align="center" id="license">License</h2>

<p>GPL-3.0-or-later. See <a href="LICENSE">LICENSE</a>.</p>

<h2 align="center" id="command-cheatsheet">Command Cheatsheet</h2>

<pre><code class="language-bash"># Sync repositories
xpm sync

# Install package(s)
xpm install &lt;pkg&gt;
xpm install &lt;pkg1&gt; &lt;pkg2&gt;

# Install without prompt
xpm install --no-confirm &lt;pkg&gt;

# Remove package(s)
xpm remove &lt;pkg&gt;

# Upgrade all installed packages
xpm upgrade

# Search package
xpm search &lt;query&gt;

# Package info
xpm info &lt;pkg&gt;

# List files owned by a package
xpm files &lt;pkg&gt;

# Query local packages
xpm query

# Show configured repos
xpm repo list
</code></pre>

<div align="center">
    <h2 id="x" align="center">X</h2>
    <p>
        <a href="https://xscriptor.io">X Web</a>
        &amp;
        <a href="https://github.com/xscriptor">X Profile</a>
    </p>
</div>
- Documentation: https://github.com/equislinux/wiki
