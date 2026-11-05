
use cosmic_text::{FontSystem, Family, Weight, Attrs};

fn main() {
    let mut fs = FontSystem::new();
    println!("Total font faces: {}", fs.db().faces().count());
    let mut adwaita_found = false;
    for face in fs.db().faces() {
        for (fam, _) in &face.families {
            if fam.to_lowercase().contains("adwaita") {
                println!("Face: {} style={:?} weight={:?}", fam, face.style, face.weight);
                adwaita_found = true;
            }
        }
    }
    if !adwaita_found {
        println!("Adwaita NOT found in fontdb!");
    }

    // Now let's query for Adwaita Sans with Weight::SEMIBOLD:
    let attrs = Attrs::new().family(Family::Name("Adwaita Sans")).weight(Weight::SEMIBOLD);
    if let Some(id) = fs.query(&attrs) {
        let face = fs.get_face(id).unwrap();
        println!("Query matched face id: {:?}, postscript: {}", id, face.post_script_name);
    } else {
        println!("Query returned None!");
    }
}
