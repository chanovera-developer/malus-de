/// Evaluador matemático simple para Spotlight (operaciones básicas +,-,*,/,^)
pub fn try_calculate(input: &str) -> Option<String> {
    let clean = input.trim();
    if clean.is_empty() || !clean.chars().any(|c| "+-*/^".contains(c)) {
        return None;
    }

    // Comprobar que solo contiene dígitos, espacios y operadores permitidos
    if !clean
        .chars()
        .all(|c| c.is_ascii_digit() || "+-*/^. ()".contains(c))
    {
        return None;
    }

    // Evaluación aritmética básica recursiva o por tokens
    eval_expression(clean).map(|res| {
        if res.fract() == 0.0 {
            format!("{}", res as i64)
        } else {
            format!("{:.4}", res)
        }
    })
}

fn eval_expression(expr: &str) -> Option<f64> {
    // Si contiene +, dividir por +
    if let Some(idx) = find_outer_op(expr, '+') {
        let (left, right) = expr.split_at(idx);
        return Some(eval_expression(left)? + eval_expression(&right[1..])?);
    }

    // Si contiene -, dividir por - (evitando números negativos iniciales)
    if let Some(idx) = find_outer_sub(expr) {
        let (left, right) = expr.split_at(idx);
        return Some(eval_expression(left)? - eval_expression(&right[1..])?);
    }

    // Si contiene *, dividir por *
    if let Some(idx) = find_outer_op(expr, '*') {
        let (left, right) = expr.split_at(idx);
        return Some(eval_expression(left)? * eval_expression(&right[1..])?);
    }

    // Si contiene /, dividir por /
    if let Some(idx) = find_outer_op(expr, '/') {
        let (left, right) = expr.split_at(idx);
        let divisor = eval_expression(&right[1..])?;
        if divisor == 0.0 {
            return None;
        }
        return Some(eval_expression(left)? / divisor);
    }

    // Número directo
    expr.trim().parse::<f64>().ok()
}

fn find_outer_op(s: &str, target: char) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => depth -= 1,
            _ if depth == 0 && c == target => return Some(i),
            _ => {}
        }
    }
    None
}

fn find_outer_sub(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => depth -= 1,
            '-' if depth == 0 && i > 0 => {
                let prev = s[..i].trim_end();
                if !prev.ends_with(['+', '-', '*', '/']) {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}
