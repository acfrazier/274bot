use super::*;

#[derive(Clone, Copy)]
enum OutsideLayoutIndex {
    EngineError,
    Unsupported(&'static str),
}

impl Exec<'_, '_> {
    pub(super) fn layout(&mut self, st: &mut State, name: &str) -> ExecResult<Step> {
        match name {
            // StringOps.ts:29-31,54-56. JavaScript length counts UTF-16 code units.
            "string_length" => {
                let text = pop_text(st)?;
                let value = match text {
                    Text::Lit(s) => {
                        Value::Known(i32::try_from(s.encode_utf16().count()).map_err(|_| {
                            unsupported(st.at, "string length exceeds engine integer")
                        })?)
                    }
                    Text::Unknown(_) => Value::Opaque(st.at),
                };
                self.push(st, value)?;
            }
            "lowercase" => {
                let text = pop_text(st)?;
                let text = match text {
                    Text::Lit(mut s) if s.is_ascii() => {
                        s.make_ascii_lowercase();
                        Text::Lit(s)
                    }
                    _ => Text::Unknown(st.at),
                };
                self.push_text(st, text)?;
            }
            // StringOps.ts:76-96 and FontType.ts:125-139. Empty text still has one line.
            "split_init" => {
                let text = pop_text(st)?;
                let args = pop_array::<3>(st)?;
                self.not_null(st, args[2])?;
                let lines = self.required(st, args[1])?;
                if lines <= 0 {
                    return Err(
                        unsupported(st.at, "split_init needs positive lines per page").into(),
                    );
                }
                st.layout.generation = st
                    .layout
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| unsupported(st.at, "layout generation overflow"))?;
                let pages = self.atom(
                    AtomKind::Layout(LayoutKind::Pages),
                    st.epoch,
                    st.layout.generation,
                    st.at,
                    CondClass::Opaque,
                    (1, i32::MAX),
                )?;
                st.layout.lines_per_page = Some(lines);
                st.layout.pages = Some(pages);
                st.layout.mesanim_may_exist = match text {
                    Text::Lit(text) => text.starts_with("<p,") && text.find('>').is_some(),
                    Text::Unknown(_) => true,
                };
            }
            // ScriptState.ts initializes splitPages to []: the positive domain starts only at init.
            "split_pagecount" => {
                let pages = st.layout.pages.unwrap_or(Value::Known(0));
                self.push(st, pages)?;
            }
            "split_linecount" => {
                let page = pop(st)?;
                self.layout_page(st, page)?;
                let count = self.layout_lines(st, page)?;
                self.push(st, count)?;
            }
            "split_get" => {
                let args = pop_array::<2>(st)?;
                self.layout_page(st, args[0])?;
                let lines = self.layout_lines(st, args[0])?;
                self.layout_index(
                    st,
                    args[1],
                    lines,
                    OutsideLayoutIndex::Unsupported(
                        "split_get line yields an unmodeled undefined string",
                    ),
                )?;
                self.push_text(st, Text::Unknown(st.at))?;
            }
            "split_getanim" => {
                let page = pop(st)?;
                if !st.layout.mesanim_may_exist {
                    self.push(st, Value::Known(-1))?;
                    return Ok(Step::Next);
                }
                let pages = st.layout.pages.unwrap_or(Value::Known(0));
                self.layout_index(
                    st,
                    page,
                    pages,
                    OutsideLayoutIndex::Unsupported("split_getanim needs an unproven page"),
                )?;
                self.push(st, Value::Opaque(st.at))?;
            }
            _ => return Err(unsupported(st.at, "unsupported layout row").into()),
        }
        Ok(Step::Next)
    }
    fn layout_page(&self, st: &State, page: Value) -> Result<(), End> {
        self.layout_index(
            st,
            page,
            st.layout.pages.unwrap_or(Value::Known(0)),
            OutsideLayoutIndex::EngineError,
        )
    }
    fn layout_index(
        &self,
        st: &State,
        index: Value,
        count: Value,
        outside: OutsideLayoutIndex,
    ) -> Result<(), End> {
        let (min, max) = domains::bounds(&self.arena, &st.domains, index);
        let (least, greatest) = domains::bounds(&self.arena, &st.domains, count);
        if min >= 0 && max < least {
            return Ok(());
        }
        let positive = min >= 0
            || domains::recorded(
                &st.conds,
                &Pred::Cmp {
                    lhs: index,
                    op: CmpOp::Ge,
                    rhs: Value::Known(0),
                },
            ) == Some(true);
        let before = domains::recorded(
            &st.conds,
            &Pred::Cmp {
                lhs: index,
                op: CmpOp::Lt,
                rhs: count,
            },
        ) == Some(true);
        if positive && before {
            return Ok(());
        }
        if max < 0 || min >= greatest {
            return Err(match outside {
                OutsideLayoutIndex::EngineError => engine(st.at, "split index outside layout"),
                OutsideLayoutIndex::Unsupported(reason) => unsupported(st.at, reason),
            });
        }
        Err(unsupported(st.at, "split index needs a range proof"))
    }
    fn layout_lines(&mut self, st: &State, page: Value) -> ExecResult<Value> {
        let pages = st
            .layout
            .pages
            .ok_or_else(|| engine(st.at, "text layout is empty"))?;
        let init = self
            .arena
            .atom_of(pages)
            .ok_or_else(|| unsupported(st.at, "layout origin missing"))?;
        let (epoch, nth, site) = (init.epoch, init.nth, init.site);
        Ok(self.atom(
            AtomKind::Layout(LayoutKind::Lines(page)),
            epoch,
            nth,
            site,
            CondClass::Opaque,
            (
                1,
                st.layout
                    .lines_per_page
                    .ok_or_else(|| unsupported(st.at, "lines per page missing"))?,
            ),
        )?)
    }
}
