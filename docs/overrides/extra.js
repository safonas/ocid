// Custom JavaScript for ocid MkDocs

// Initialize Mermaid after page load
document.addEventListener("DOMContentLoaded", function() {
    // Re-render Mermaid diagrams (in case they didn't render properly)
    if (typeof mermaid !== "undefined") {
        mermaid.initialize({
            theme: "base",
            themeVariables: {
                primaryColor: "#673ab7",
                primaryTextColor: "#fff",
                lineColor: "#673ab7",
                secondaryColor: "#3f51b5",
                tertiaryColor: "#2196f3"
            }
        });

        // Find all Mermaid diagrams and re-render them
        const diagrams = document.querySelectorAll(".mermaid");
        diagrams.forEach(function(diagram) {
            const code = diagram.textContent;
            diagram.innerHTML = "";
            mermaid.mermaidAPI.render(
                "mermaid-" + Math.random().toString(36).substr(2, 9),
                code,
                function(svgCode) {
                    diagram.innerHTML = svgCode;
                }
            );
        });
    }

    // Add copy button to code blocks
    const codeBlocks = document.querySelectorAll("pre code");
    codeBlocks.forEach(function(codeBlock) {
        const pre = codeBlock.parentElement;
        if (pre.classList.contains("copy")) return; // Already has copy button

        const copyButton = document.createElement("button");
        copyButton.className = "copy-button";
        copyButton.innerHTML = "Copy";
        copyButton.style.position = "absolute";
        copyButton.style.right = "8px";
        copyButton.style.top = "8px";
        copyButton.style.background = "#3182ce";
        copyButton.style.color = "white";
        copyButton.style.border = "none";
        copyButton.style.borderRadius = "4px";
        copyButton.style.padding = "4px 8px";
        copyButton.style.cursor = "pointer";
        copyButton.style.fontSize = "0.8em";

        pre.style.position = "relative";
        pre.appendChild(copyButton);

        copyButton.addEventListener("click", function() {
            const code = codeBlock.textContent;
            navigator.clipboard.writeText(code).then(function() {
                copyButton.innerHTML = "Copied!";
                setTimeout(function() {
                    copyButton.innerHTML = "Copy";
                }, 2000);
            });
        });
    });

    // Smooth scrolling for anchor links
    document.querySelectorAll("a[href^='#']").forEach(function(anchor) {
        anchor.addEventListener("click", function(e) {
            e.preventDefault();
            const target = document.querySelector(this.getAttribute("href"));
            if (target) {
                target.scrollIntoView({
                    behavior: "smooth",
                    block: "start"
                });
            }
        });
    });

    // Add "Back to Top" button
    const backToTopButton = document.createElement("button");
    backToTopButton.innerHTML = "↑ Back to Top";
    backToTopButton.style.position = "fixed";
    backToTopButton.style.bottom = "20px";
    backToTopButton.style.right = "20px";
    backToTopButton.style.background = "#3182ce";
    backToTopButton.style.color = "white";
    backToTopButton.style.border = "none";
    backToTopButton.style.borderRadius = "50%";
    backToTopButton.style.width = "50px";
    backToTopButton.style.height = "50px";
    backToTopButton.style.cursor = "pointer";
    backToTopButton.style.boxShadow = "0 2px 4px rgba(0, 0, 0, 0.2)";
    backToTopButton.style.display = "none";
    backToTopButton.style.zIndex = "1000";

    document.body.appendChild(backToTopButton);

    backToTopButton.addEventListener("click", function() {
        window.scrollTo({
            top: 0,
            behavior: "smooth"
        });
    });

    window.addEventListener("scroll", function() {
        if (window.pageYOffset > 300) {
            backToTopButton.style.display = "block";
        } else {
            backToTopButton.style.display = "none";
        }
    });
});

// Highlight current navigation item
window.addEventListener("scroll", function() {
    const sections = document.querySelectorAll("h2, h3");
    const navLinks = document.querySelectorAll(".md-nav__link");

    let current = "";
    sections.forEach(function(section) {
        const sectionTop = section.offsetTop;
        const sectionHeight = section.clientHeight;
        if (window.pageYOffset >= sectionTop - 200) {
            current = section.getAttribute("id");
        }
    });

    navLinks.forEach(function(link) {
        link.classList.remove("active");
        if (link.getAttribute("href") === "#" + current) {
            link.classList.add("active");
        }
    });
});
