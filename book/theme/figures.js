// A diagram opens on its own when clicked, so it can be read at full size on a phone.
document.querySelectorAll('figure > img[src$=".svg"]').forEach((diagram) => {
    diagram.style.cursor = "zoom-in";
    diagram.title = "Open at full size";
    diagram.addEventListener("click", () => window.open(diagram.src, "_blank"));
});
